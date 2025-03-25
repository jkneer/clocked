import math


def main():
    n = 60
    phase = -90
    angle = 360 / n / 2
    rotangle = angle
    ampl = 500
    for i in range(1, n + 1):
        rad = math.radians(angle + phase)
        pos = (ampl * math.sin(rad), ampl * math.cos(rad))
        print(
            f'{{ "type": "wokwi-neopixel", "id": "rgb{i}", "top": {pos[0]:.1f}, "left": {pos[1]:.1f}, "rotate": {angle - 180}, "attrs": {{ }} }},'
        )

        if i % 2:
            rotangle += 360 / n * 2
        angle += 360 / n

    for i in range(1, n):
        print(f'[ "rgb{i}:DOUT", "rgb{i + 1}:DIN", "green", [ "h0" ] ],')


if __name__ == "__main__":
    main()
