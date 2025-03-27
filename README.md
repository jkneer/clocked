# clocked
A clock build with RGB light indicators.

https://github.com/user-attachments/assets/19da2db1-5a2d-457d-999b-6be772b56d4c

Aside from being just a cool addon to a classical analogue clock. We pursue two educational goals with clocked
 - Clocked helps young children learn reading the clock while also teaching time management.
 - Clocked is an interesting device to teach young adolescents microcontroller principles.

## FAQ
1. I got a clocked during a class session and want the official firmware
   You need to flash the clock. Currently there is not released firmware, once there is, instructions will be published alongside.


## target audience
The project is targetted to parents and teachers 


## user audience
Children 
# hardware
The mechanical CAD work is done in onshape and can be found here: [mechanical design files](https://cad.onshape.com/documents/c5d2af0e8c6398f21e146574/w/c31c92c8e362af9e481c19e6/e/3c8fc13ddff8f8868382d92b)
The current version of the design can be found in the hardware directory. There are two parts, the "mirror" and an adapter to the analog clock used. For the prototype we have used an IKEA Trömma analog clock. If you want to use a differen clock that has a smaller diameter than the mirror, you only need to design an adapter to hold the mirror in place.

## LED strip
We are using a ws2812b strip with 74LED per meter from bft lighting (we use 60 LED, one for each minute). There is a mirror design for 144LED per meter (2 LED per minute pocket) on onshape. But that design is not as finely tuned as the 60 LED one. We had some reservations on potential power draw when all LEDs are turned on.

## Processor
We use a esp32-s3 mini board. Using a USB-C Board should give a power budget of 15W (given the power supply) without any power delivery negotiation necessary via USB.

## Level Shift
We use a low cost 4 channel level shift to get the data signal for the LED strip from the processors native 3,3V to a 5V level.

# software

## BOM
 - 74LED/m RGB LED stripe with ws2812a/b driver
 - esp32-s2 mini board
 - 4 channel level shifter
 - Ikea Trömma Clock

## License
The software of this project is licensed under the Apache License (version 2.0) or the MIT license at your choice.
The hardware of this project is licensed under the permissive CERN OpenHardware license (CERN-OHL-W-2.0)

Unless you explicitly state otherwise, any software or documentation contribution intentionally submitted for inclusion in the clocked project by you, as defined in the Apache-2.0 license, shall be dually licensed as above, without any additional terms or conditions.
Any intentional contribution to the hardware design should be licensed under the CERN OHL-W-V2.0 unless you explicitely state otherwise.

See [LICENSE-Apache](LICENSE-Apache.txt), [LICENSE-MIT](LICENSE-MIT.txt), and
[LICENSE-CERN-OHL-W-20](LICENSE-CERN-OHL-W-20.txt) for details.
