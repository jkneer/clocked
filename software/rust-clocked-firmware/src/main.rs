#![no_std]
#![no_main]

use core::net::{IpAddr, SocketAddr};

use alloc::string::ToString;
use embassy_executor::Spawner;
use embassy_net::{
    dns::DnsQueryType,
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpMetadata, UdpSocket},
    IpAddress, IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
};
use embassy_time::{Duration, Timer, WithTimeout};

use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    peripherals,
    rmt::Rmt,
    rng::Rng,
    rtc_cntl::Rtc,
    time::Rate,
    timer::{systimer::SystemTimer, timg::TimerGroup},
};

use esp_hal_smartled::{smartLedBuffer, SmartLedsAdapter};

use esp_wifi::{
    init,
    wifi::{ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiState},
    EspWifiController,
};

use smart_leds::{
    brightness, gamma,
    hsv::{hsv2rgb, Hsv},
    SmartLedsWrite,
};

use chrono::{DateTime, NaiveDateTime};
use sntpc::{fraction_to_microseconds, get_time, NtpContext, NtpTimestampGenerator};

// use defmt::{debug, error, info, warn};
// use defmt_rtt as _;
use esp_backtrace as _;
use esp_println as _;
use esp_println::{
    logger::{init_logger, init_logger_from_env},
    println,
};
use log::{debug, error, info, warn, LevelFilter};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

const POOL_NTP_ADDR: &str = "pool.ntp.org";
const NTP_RETRY_TIMEOUT: u16 = 15;
const NTP_RETRIEVAL_INTERVAL: u16 = 3600;

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

#[derive(Copy, Clone, Default)]
struct TimestampGen {
    duration: u64,
}

impl NtpTimestampGenerator for TimestampGen {
    fn init(&mut self) {
        self.duration = 0u64;
    }

    fn timestamp_sec(&self) -> u64 {
        self.duration >> 32
    }

    fn timestamp_subsec_micros(&self) -> u32 {
        (self.duration & 0xff_ff_ff_ffu64) as u32
    }
}

// Define a static mutable variable to hold your stack.
// Note: using `static mut` is inherently unsafe; if you need concurrent access,
// consider using a mutex or another safe abstraction.
static mut NET_STACK: MaybeUninit<Stack> = MaybeUninit::uninit();

// #[panic_handler]
// fn panic(_: &core::panic::PanicInfo) -> ! {
//     loop {}
// }

extern crate alloc;
use core::mem::MaybeUninit;

fn init_heap() {
    const HEAP_SIZE: usize = 32 * 1024;
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();

    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            HEAP.as_mut_ptr() as *mut u8,
            HEAP_SIZE,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    info!("start connection task");
    info!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_wifi::wifi::wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            info!("Starting wifi");
            controller.start_async().await.unwrap();
            info!("Wifi started!");
        }
        info!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn ntp_sync_task(stack: &'static embassy_net::Stack<'static>) {
    info!(target: "NTP", "Started NTP task");
    // We'll use a 48-byte buffer for our NTP packets.
    let mut ntp_packet = [0u8; 48];
    // The first byte (LI, VN, Mode) should be set to 0x1B
    // (LI = 0, VN = 3, Mode = 3 => client request).
    ntp_packet[0] = 0x1B;

    // Create separate buffers for the UDP socket.
    let mut rx_buffer = [0u8; 256];
    let mut tx_buffer = [0u8; 256];

    // Create metadata buffers.
    // Here we assume two entries are sufficient (adjust the size as needed).
    let mut rx_meta: [PacketMetadata; 2] = unsafe { MaybeUninit::zeroed().assume_init() };
    let mut tx_meta: [PacketMetadata; 2] = unsafe { MaybeUninit::zeroed().assume_init() };

    // Now create the UDP socket with all required arguments.
    let mut socket = UdpSocket::new(
        *stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );

    // Bind the socket to a local port (e.g., 12345).
    socket.bind(12345).expect("Unable to create UDP socket");
    info!(target: "NTP", "Bound UDP socket");

    let context = NtpContext::new(TimestampGen::default());

    // see readme
    loop {
        // You can use an IP address for a known NTP server.
        // For example, time.nist.gov (129.6.15.28) on port 123:
        let ntp_addr = match stack
            .dns_query(POOL_NTP_ADDR, DnsQueryType::A)
            .with_timeout(Duration::from_secs(2))
            .await
        {
            Ok(response) => match response {
                Ok(addrs) => {
                    if addrs.is_empty() {
                        unreachable!("Should not happen, otherwise we get an error?")
                    }
                    debug!(target: "NTP", "Resolved {} to {:?}", POOL_NTP_ADDR, addrs);
                    addrs[0]
                }
                Err(e) => {
                    info!(target: "NTP", "DNS request error ({:?}), retry in {}s", e, NTP_RETRY_TIMEOUT);
                    Timer::after(Duration::from_secs(NTP_RETRY_TIMEOUT as u64)).await;
                    continue;
                }
            },
            Err(e) => {
                info!(target: "NTP", "DNS request timeout ({:?}), retry in {}s", e, NTP_RETRY_TIMEOUT);
                Timer::after(Duration::from_secs(NTP_RETRY_TIMEOUT as u64)).await;
                continue;
            }
        };

        let ntp_addr: IpAddr = ntp_addr.into();
        info!(target: "NTP", "Address of NTP server {ntp_addr}");
        let time = match get_time(
            SocketAddr::from((ntp_addr, 123u16)).into(),
            &socket,
            context,
        )
        .with_timeout(Duration::from_secs(5))
        .await
        {
            Ok(response) => match response {
                Ok(time) => {
                    println!("NTP:: answer");
                    assert_ne!(time.sec(), 0);
                    let seconds = time.sec();
                    let microseconds = fraction_to_microseconds(time.sec_fraction());
                    let time =
                        DateTime::from_timestamp_micros(seconds as i64 + microseconds as i64)
                            .unwrap()
                            .naive_local();
                    info!("{:?}", time);
                    time
                }
                Err(e) => {
                    info!(target: "NTP", "NTP request error ({:?}), retry in {}s", e, NTP_RETRY_TIMEOUT);
                    Timer::after(Duration::from_secs(NTP_RETRY_TIMEOUT as u64)).await;
                    continue;
                }
            },
            Err(e) => {
                info!(target: "NTP", "NTP request timeout ({:?}), retry in {}s", e, NTP_RETRY_TIMEOUT);
                Timer::after(Duration::from_secs(NTP_RETRY_TIMEOUT as u64)).await;
                continue;
            }
        };

        // for addr in POOL_NTP_ADDR.to_socket_addrs() {}

        // for addr in stack.dns_query("pool.ntp.org", DnsQueryType::A).await {}
        // let ntp_server = (ntp_server, 123);
        // println!("NTP server: {:?}", ntp_server);

        // let ntp_context = NtpContext::new(StdTimestampGen::default());
        // let result = get_time(ntp_server, &socket, ntp_context);

        // match result {
        //     Ok(time) => {
        //         assert_ne!(time.sec(), 0);
        //         let seconds = time.sec();
        //         let microseconds = u64::from(time.sec_fraction()) * 1_000_000 / u64::from(u32::MAX);
        //         println!("Got time from [{POOL_NTP_ADDR}] {ntp_server}: {seconds}.{microseconds}");

        //         break;
        //     }
        //     Err(err) => println!("Err: {err:?}"),
        // }

        /*
        // (Re)initialize the request packet in case it was overwritten.
        for b in ntp_packet.iter_mut() {
            *b = 0;
        }
        ntp_packet[0] = 0x1B; // Set LI, VN, Mode.

        // Send the request to the NTP server.
        if let Err(e) = socket.send_to(&ntp_packet, ntp_server).await {
            println!("NTP send error: {:?}", e);
            Timer::after(Duration::from_secs(30)).await;
            continue;
        }
        println!("NTP: Send request");

        // Wait for the response.
        match socket.recv_from(&mut ntp_packet).await {
            Ok((n, _addr)) => {
                if n < 48 {
                    println!("NTP response too short: {} bytes", n);
                } else {
                    // The transmit timestamp is at offset 40 and is 4 bytes.
                    let secs = u32::from_be_bytes([
                        ntp_packet[40],
                        ntp_packet[41],
                        ntp_packet[42],
                        ntp_packet[43],
                    ]);
                    // NTP time starts on 1900-01-01, Unix time on 1970-01-01.
                    // The difference is 2,208,988,800 seconds.
                    let ntp_to_unix_offset = 2_208_988_800u32;
                    let unix_time = secs.saturating_sub(ntp_to_unix_offset);
                    println!(
                        "NTP time: {} seconds since Unix epoch - {}",
                        unix_time,
                        NaiveDateTime::from
                    );
                    //println!("NTP:: current internal time {}", rtc.current_time());

                    // *** Update your RTC here ***
                    // If you have an API to update the internal RTC, call it with unix_time.
                    // For example:
                    // rtc.set_time(unix_time);
                }
            }
            Err(e) => {
                println!("NTP receive error: {:?}", e);
            }
        }*/

        // Wait for a minute (or your desired interval) before re-syncing.
        Timer::after(Duration::from_secs(NTP_RETRIEVAL_INTERVAL as u64)).await;
    }
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    init_logger(LevelFilter::Debug);
    //init_logger_from_env();
    // generator version: 0.3.1
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 128 * 1024);

    let timg1 = TimerGroup::new(peripherals.TIMG1);
    let mut rng = Rng::new(peripherals.RNG);

    let rtc = Rtc::new(peripherals.LPWR);
    println!("Current processor time {}", rtc.current_time());
    //rtc.set_current_time(current_time);

    let esp_wifi_ctrl = &*mk_static!(
        EspWifiController<'static>,
        init(timg1.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap()
    );

    let (controller, interfaces) = esp_wifi::wifi::new(&esp_wifi_ctrl, peripherals.WIFI).unwrap();

    let wifi_interface = interfaces.sta;

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    info!("Embassy initialized!");

    let config = embassy_net::Config::dhcpv4(Default::default());

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // Store the stack in the static variable.
    unsafe {
        NET_STACK.write(stack);
    }

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(runner)).ok();

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    // Spawn the NTP sync task.
    // spawner.spawn(ntp_sync_task(&stack)).ok();
    spawner
        .spawn(ntp_sync_task(unsafe { NET_STACK.assume_init_ref() }))
        .ok();

    // loop {
    //     Timer::after(Duration::from_millis(1_000)).await;

    //     let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

    //     socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    //     let remote_endpoint = (Ipv4Addr::new(142, 250, 185, 115), 80);
    //     println!("connecting...");
    //     let r = socket.connect(remote_endpoint).await;
    //     if let Err(e) = r {
    //         println!("connect error: {:?}", e);
    //         continue;
    //     }
    //     println!("connected!");
    //     let mut buf = [0; 1024];
    //     loop {
    //         use embedded_io_async::Write;
    //         let r = socket
    //             .write_all(b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n")
    //             .await;
    //         if let Err(e) = r {
    //             println!("write error: {:?}", e);
    //             break;
    //         }
    //         let n = match socket.read(&mut buf).await {
    //             Ok(0) => {
    //                 println!("read EOF");
    //                 break;
    //             }
    //             Ok(n) => n,
    //             Err(e) => {
    //                 println!("read error: {:?}", e);
    //                 break;
    //             }
    //         };
    //         println!("{}", core::str::from_utf8(&buf[..n]).unwrap());
    //     }
    //     Timer::after(Duration::from_millis(3000)).await;
    // }

    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();

    let rmt_buffer = smartLedBuffer!(60);
    let mut led = SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO2, rmt_buffer);
    //let delay = Delay::new();

    // Create a buffer of 144 LED colors initialized to black.
    let black = hsv2rgb(Hsv {
        hue: 0,
        sat: 0,
        val: 0,
    });
    let mut data = [black; 60];
    let mut index = 0;
    let mut direction = 1;
    let light_length = 1;
    let mut color = Hsv {
        hue: 200,
        sat: 255,
        val: 255,
    };

    // TODO: Spawn some tasks
    //let _ = spawner;

    loop {
        // Clear the LED buffer.
        for led_pixel in data.iter_mut() {
            *led_pixel = black;
        }

        // // Set a block of 5 LEDs starting at 'index' to the desired color.
        // for offset in 0..light_length {
        //     let pos = (index + offset) as usize;
        //     if pos < data.len() {
        //         data[pos] = hsv2rgb(color);
        //     }
        // }
        // Set a block of 5 LEDs starting at 'index' to the desired color, wrapping around if needed.
        for offset in 0..light_length {
            let pos = (index + offset) % data.len();
            data[pos] = hsv2rgb(color);
        }

        // Write the LED data.
        led.write(data.iter().cloned()).unwrap();

        // Delay between frames (adjust as necessary).
        //delay.delay_millis(20u32);
        Timer::after(Duration::from_millis(1000)).await;

        // Increment the index and wrap around automatically.
        index = (index + 1) % data.len();

        // // Update the index and reverse direction if at either end.
        // if index <= 0 {
        //     direction = 1;
        // } else if index >= (data.len() as i32 - light_length) {
        //     direction = -1;
        // }
        // index += direction;

        color.hue = (color.hue + 1) % 255;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-beta.0/examples/src/bin
}
