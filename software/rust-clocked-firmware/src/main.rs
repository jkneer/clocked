#![no_std]
#![no_main]

use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use embassy_net::{
    tcp::TcpSocket, udp::PacketMetadata, udp::UdpMetadata, udp::UdpSocket, IpListenEndpoint,
    Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
};
use embassy_time::{Duration, Timer};

use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    rmt::Rmt,
    rng::Rng,
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

use defmt::info;
use esp_println as _;
use esp_println::println;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

// Define a static mutable variable to hold your stack.
// Note: using `static mut` is inherently unsafe; if you need concurrent access,
// consider using a mutex or another safe abstraction.
static mut NET_STACK: MaybeUninit<Stack> = MaybeUninit::uninit();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

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
    info!("entered npt task");
    println!("Entered ntp task");
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
    socket.bind(12345).unwrap();

    // You can use an IP address for a known NTP server.
    // For example, time.nist.gov (129.6.15.28) on port 123:
    let ntp_server: (Ipv4Addr, u16) = (Ipv4Addr::new(129, 6, 15, 28), 123);

    loop {
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
                    println!("NTP time: {} seconds since Unix epoch", unix_time);

                    // *** Update your RTC here ***
                    // If you have an API to update the internal RTC, call it with unix_time.
                    // For example:
                    // rtc.set_time(unix_time);
                }
            }
            Err(e) => {
                println!("NTP receive error: {:?}", e);
            }
        }

        // Wait for a minute (or your desired interval) before re-syncing.
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.3.1

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 60 * 1024);

    let timg1 = TimerGroup::new(peripherals.TIMG1);
    let mut rng = Rng::new(peripherals.RNG);

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
    // Spawn the NTP sync task.
    // spawner.spawn(ntp_sync_task(&stack)).ok();
    spawner
        .spawn(ntp_sync_task(unsafe { NET_STACK.assume_init_ref() }))
        .ok();

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
    let delay = Delay::new();

    // Create a buffer of 144 LED colors initialized to black.
    let black = hsv2rgb(Hsv {
        hue: 0,
        sat: 0,
        val: 0,
    });
    let mut data = [black; 60];
    let mut index = 0;
    let mut direction = 1;
    let light_length = 5;
    let mut color = Hsv {
        hue: 200,
        sat: 255,
        val: 255,
    };

    // TODO: Spawn some tasks
    let _ = spawner;

    // loop {
    //     // Clear the LED buffer.
    //     for led_pixel in data.iter_mut() {
    //         *led_pixel = black;
    //     }

    //     // Set a block of 5 LEDs starting at 'index' to the desired color.
    //     for offset in 0..light_length {
    //         let pos = (index + offset) as usize;
    //         if pos < data.len() {
    //             data[pos] = hsv2rgb(color);
    //         }
    //     }

    //     // Write the LED data.
    //     led.write(data.iter().cloned()).unwrap();

    //     // Delay between frames (adjust as necessary).
    //     delay.delay_millis(20u32);

    //     // Update the index and reverse direction if at either end.
    //     if index <= 0 {
    //         direction = 1;
    //     } else if index >= (data.len() as i32 - light_length) {
    //         direction = -1;
    //     }
    //     index += direction;

    //     color.hue = (color.hue + 1) % 255;
    // }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-beta.0/examples/src/bin
}
