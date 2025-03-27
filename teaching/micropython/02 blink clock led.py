import machine
import neopixel
import time

np = neopixel.NeoPixel(machine.Pin(1), 60)  # 1 LED

while True:
    np[0] = (255, 0, 0)  # Red
    np.write()
    time.sleep(1)
    np[0] = (0, 0, 0)  # Off
    np.write()
    time.sleep(1)
