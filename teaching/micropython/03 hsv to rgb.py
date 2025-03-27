import machine
import neopixel

np = neopixel.NeoPixel(machine.Pin(1), 60)  # 1 LED


def hsv_to_rgb(h, s, v):
    if s == 0:
        return v, v, v

    region = h // 43
    remainder = (h - (region * 43)) * 6

    p = (v * (255 - s)) >> 8
    q = (v * (255 - ((s * remainder) >> 8))) >> 8
    t = (v * (255 - ((s * (255 - remainder)) >> 8))) >> 8

    if region == 0:
        return v, t, p
    elif region == 1:
        return q, v, p
    elif region == 2:
        return p, v, t
    elif region == 3:
        return p, q, v
    elif region == 4:
        return t, p, v
    else:
        return v, p, q


r, g, b = hsv_to_rgb(128, 255, 128)  # Some greenish tone
np[0] = (r, g, b)
np.write()
