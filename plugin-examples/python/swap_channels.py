"""Example Artwork Creator plugin in Python: swaps two color channels of an image. Plain
Python, no package needed.

`make plugins` copies it into plugins/. See "Plugins" in README.md.
"""

from artwork_plugin_sdk import Effect, Plugin, choice_param

# Offsets of the channels in a pixel.
CHANNELS = {"red": 0, "green": 1, "blue": 2}


class SwapChannels(Effect):
    id = "pyswap.swap"
    name = "Swap channels"
    description = "Swap two color channels (Python example plugin)"
    params = [
        choice_param("pair", "Swap", [("red-blue", "Red and blue"),
                                      ("red-green", "Red and green"),
                                      ("green-blue", "Green and blue")], "red-blue"),
    ]

    def apply(self, image, values, scale):
        a, b = (CHANNELS[c] for c in values["pair"].split("-"))
        px = image.pixels
        px[a::4], px[b::4] = px[b::4], px[a::4]


plugin = Plugin("Swap channels", "1.0", effects=[SwapChannels()])
