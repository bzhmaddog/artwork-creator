"""Example Artwork Creator plugin in Python: a posterize effect, reducing each color
channel to a few levels, and a bands generator filling a shape with stripes of two
colors at any angle. Plain Python, no package needed.

`make plugins` copies it into plugins/. See "Plugins" in README.md.
"""

import math

from artwork_plugin_sdk import Effect, Generator, Plugin, color, color_param, float_param, int_param


class Posterize(Effect):
    id = "pyexample.posterize"
    name = "Posterize"
    description = "Reduce each color channel to a few levels (Python example plugin)"
    params = [int_param("levels", "Levels", 2, 32, 4)]

    def apply(self, image, values, scale):
        step = 255 / (values["levels"] - 1)
        table = bytes(round(round(v / step) * step) for v in range(256))
        # Every byte goes through the table; the alpha bytes are put back after.
        alpha = image.pixels[3::4]
        image.pixels[:] = image.pixels.translate(table)
        image.pixels[3::4] = alpha


class Bands(Generator):
    id = "pyexample.bands"
    name = "Bands"
    description = "Stripes of two colors (Python example plugin)"
    params = [
        float_param("width", "Band width", 0.5, 100, 5, unit=" mm"),
        # A float in degrees gets a dial: counterclockwise, 0 for horizontal bands.
        float_param("angle", "Orientation", -180, 180, 0, unit="°", decimals=0),
        color_param("first", "First color", "#1d3557ff"),
        color_param("second", "Second color", "#f1faeeff"),
    ]

    def generate(self, image, values, px_per_mm):
        band = max(values["width"] * px_per_mm, 1)
        turn = math.radians(values["angle"])
        # Across the bands, in band widths: at pixel x of a row, start + x * step (y is
        # down, so the bands turn counterclockwise on the page).
        step = math.sin(turn) / band
        colors = [bytes(color(values[k])) for k in ("first", "second")]
        out = bytearray()
        for y in range(image.height):
            start = (0.5 * math.sin(turn) + (y + 0.5) * math.cos(turn)) / band
            # A row is drawn as runs of one band each, not pixel by pixel.
            x = 0
            while x < image.width:
                k = math.floor(start + x * step)
                if step > 0:
                    end = math.ceil((k + 1 - start) / step)
                elif step < 0:
                    end = math.floor((k - start) / step) + 1
                else:
                    end = image.width
                end = min(image.width, max(end, x + 1))
                out += colors[k % 2] * (end - x)
                x = end
        image.pixels[:] = out


plugin = Plugin("Python example", "1.0", effects=[Posterize()], generators=[Bands()])
