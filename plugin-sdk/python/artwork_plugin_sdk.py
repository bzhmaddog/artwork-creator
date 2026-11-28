"""Artwork Creator plugins in Python.

A Python plugin is one .py file in the app's plugins folder. It defines `plugin`, a
`Plugin` with its effects and generators::

    from artwork_plugin_sdk import Effect, Plugin, int_param

    class Invert(Effect):
        id = "example.invert"
        name = "Invert"
        params = [int_param("amount", "Amount", 0, 100, 100, unit=" %")]

        def apply(self, image, values, scale):
            ...  # change image.pixels in place

    plugin = Plugin("Example", "1.0", effects=[Invert()],
                    translations={"fr-FR": {"Invert": "Inverser", "Amount": "Quantité"}})

`translations` maps language codes to catalogs of the English texts of the code (names,
descriptions, parameter labels, groups and choice labels) to that language's; units are
not translated. The app shows the texts of its language, the closest one there is
(fr-FR for fr-CA), else the English ones. `plugin.untranslated("fr-FR")` tells a
plugin's tests what a catalog lacks.

The app runs the file in a Python process of its own, with this module importable as
`artwork_plugin_sdk`, and keeps that process running: a module-level variable lives as long
as the app. It talks to the plugin over stdin and stdout, so the plugin must not use
them; `print` goes to stderr, which the app writes to its log.

Pixels are straight (not premultiplied) RGBA, 8 bits per channel, rows top to bottom, in
`image.pixels`, a bytearray of `image.width * image.height * 4` bytes. With numpy:
`numpy.frombuffer(image.pixels, numpy.uint8).reshape(image.height, image.width, 4)` is
a writable view of them.

Parameter values come as a dict holding every parameter, already checked against its
range: a float or int is a number, a bool a bool, a color a "#rrggbbaa" string (see
`color`), a choice one of its option ids.

The protocol, for other languages: the app writes one line of JSON (a request), followed
by `width * height * 4` bytes of pixels for "apply"; the plugin answers one line of JSON,
`{"ok": true, ...}` or `{"ok": false, "error": "..."}`, followed by the pixels when
"apply" or "generate" succeeded. Requests: `{"op": "describe", "language": "fr-FR"}`,
answered with the name, version, effects and generators, their texts in that language
(the English ones without "language", or for a language it has none for); `{"op": "padding", "id", "values"}`, answered with
`"padding": [left, top, right, bottom]`; `{"op": "apply", "id", "values", "width",
"height", "scale"}`; `{"op": "generate", "id", "values", "width", "height",
"px_per_mm"}`.
"""

import json
import math
import runpy
import sys

# The SDK's version, the same as the artwork-plugin-sdk crate's.
__version__ = "0.6.0"


def _with(param, role=None, group=None):
    """`param` with its optional keys, those given."""
    extra = {k: v for k, v in (("role", role), ("group", group)) if v}
    return dict(param, **extra)


# Every *_param helper takes `group`: the name of a folding group. The parameters with the
# same one are shown together under it, where the first of them is; without one, a
# parameter has a row of its own.


def float_param(id, label, min, max, default, unit="", decimals=1, role=None, group=None):
    """A number with decimals; `unit` is shown after it, e.g. " px". `role` says what it
    is for, which picks its control: "angle" (degrees, counterclockwise from the right:
    a value and a dial) or "size" (a length in the image's pixels: fine steps for small
    values, shown in mm on layers whose pixels have a size on the page). Without one, a
    float in "°" is taken for an angle and one in "px" for a size."""
    return _with({"id": id, "label": label, "type": "float", "min": min, "max": max,
                  "default": default, "unit": unit, "decimals": decimals}, role, group)


def int_param(id, label, min, max, default, unit="", role=None, group=None):
    """A whole number. For a random variation, use `seed_param`; without a role, one
    with the id "seed" is taken for one."""
    return _with({"id": id, "label": label, "type": "int", "min": min, "max": max,
                  "default": default, "unit": unit}, role, group)


# The largest value of a seed_param.
SEED_MAX = 2**31 - 1


def seed_param(id, label, group=None):
    """A seed: a random variation the app picks, when the effect is added and with a
    regenerate button shown in place of a slider, from 0 to SEED_MAX. Seed a random
    generator with it, so the same value gives the same result on every render."""
    return int_param(id, label, 0, SEED_MAX, 0, role="seed", group=group)


def bool_param(id, label, default, group=None):
    """A check box."""
    return _with({"id": id, "label": label, "type": "bool", "default": default}, group=group)


def color_param(id, label, default, group=None):
    """A color; `default` is "#rrggbbaa", "#rrggbb" or an (r, g, b, a) tuple."""
    if not isinstance(default, str):
        default = "#%02x%02x%02x%02x" % tuple(default)
    return _with({"id": id, "label": label, "type": "color", "default": default}, group=group)


def choice_param(id, label, options, default, group=None):
    """One of several options, given as (id, label) pairs; the value is the id."""
    return _with({"id": id, "label": label, "type": "choice", "default": default,
                  "options": [{"id": o, "label": text} for o, text in options]}, group=group)


def color(value):
    """The (r, g, b, a) tuple of a "#rrggbbaa" or "#rrggbb" color value."""
    s = value.lstrip("#")
    if len(s) == 6:
        s += "ff"
    return tuple(int(s[i:i + 2], 16) for i in range(0, 8, 2))


class Image:
    """Straight RGBA pixels: `pixels` holds `height` rows of `width * 4` bytes."""

    def __init__(self, width, height, pixels=None):
        self.width = width
        self.height = height
        self.pixels = bytearray(width * height * 4) if pixels is None else pixels


class Effect:
    """An image effect. Set `id` (unique and stable: it is saved in projects; prefix it
    with something of your plugin's), `name`, `description` and `params`, and
    write `apply`."""

    id = ""
    name = ""
    description = ""
    params = []

    def padding(self, values):
        """Room the effect draws into around the image (left, top, right, bottom), in
        source pixels; None (the default) if it never draws outside it."""
        return None

    def apply(self, image, values, scale):
        """Changes `image` in place. It already has the margins asked for by
        `padding`. `scale` is its size relative to the layer's source: multiply
        distances in source pixels by it. Raise an exception to report an error."""
        raise NotImplementedError


class Generator:
    """Draws a shape's fill. Set `id`, `name`, `description` and `params` as for
    an effect, and write `generate`."""

    id = ""
    name = ""
    description = ""
    params = []

    def generate(self, image, values, px_per_mm):
        """Draws into `image`, transparent at first, which covers the shape's box at
        `px_per_mm` pixels per mm: multiply lengths in mm by it. The same values must
        give the same picture at any resolution."""
        raise NotImplementedError


class Plugin:
    """What the plugin file provides, as its `plugin` variable."""

    def __init__(self, name, version, effects=(), generators=(), translations=None):
        self.name = name
        self.version = version
        self.effects = list(effects)
        self.generators = list(generators)
        self.translations = dict(translations or {})

    def _catalog(self, language):
        """The catalog of `language`, the closest one there is (same code, else same
        language), or {}. U.S. English, the language of the code's texts, takes none
        but its own: another English one (en-GB) would show its spellings."""
        if not language:
            return {}

        def norm(code):
            return code.replace("_", "-").lower()

        want = norm(language)
        for code, catalog in self.translations.items():
            if norm(code) == want:
                return catalog
        if want == "en-us":
            return {}
        for code, catalog in self.translations.items():
            if norm(code).split("-")[0] == want.split("-")[0]:
                return catalog
        return {}

    def _texts(self, x):
        """The English texts of an effect or generator, to translate."""
        yield x.name
        if x.description:
            yield x.description
        for p in x.params:
            yield p["label"]
            if p.get("group"):
                yield p["group"]
            for o in p.get("options", ()):
                yield o["label"]

    def untranslated(self, language):
        """The English texts that the catalog of `language` doesn't translate, each once."""
        catalog = self._catalog(language)
        missing = []
        for x in self.effects + self.generators:
            for text in self._texts(x):
                if not catalog.get(text) and text not in missing:
                    missing.append(text)
        return missing

    def describe(self, language=None):
        catalog = self._catalog(language)

        def tr(text):
            return catalog.get(text) or text

        def param(p):
            p = dict(p, label=tr(p["label"]))
            if p.get("group"):
                p["group"] = tr(p["group"])
            if "options" in p:
                p["options"] = [dict(o, label=tr(o["label"])) for o in p["options"]]
            return p

        def entry(x):
            return {"id": x.id, "name": tr(x.name), "description": tr(x.description),
                    "params": [param(p) for p in x.params]}
        return {
            "name": self.name,
            "version": self.version,
            "effects": [dict(entry(e), padding=type(e).padding is not Effect.padding)
                        for e in self.effects],
            "generators": [entry(g) for g in self.generators],
        }

    def handle(self, request, data):
        """The answer to one request, and the pixels to send after it (or None)."""
        op = request.get("op")
        if op == "describe":
            return dict(self.describe(request.get("language")), ok=True), None
        values = request.get("values", {})
        if op in ("padding", "apply"):
            effect = self._find(self.effects, request.get("id"))
            if op == "padding":
                pad = effect.padding(values) or (0, 0, 0, 0)
                pad = [float(p) for p in pad]
                # JSON has no NaN or infinity.
                return {"ok": True, "padding": [p if math.isfinite(p) else 0.0 for p in pad]}, None
            image = Image(request["width"], request["height"], data)
            effect.apply(image, values, request["scale"])
            return {"ok": True}, image.pixels
        if op == "generate":
            generator = self._find(self.generators, request.get("id"))
            image = Image(request["width"], request["height"])
            generator.generate(image, values, request["px_per_mm"])
            return {"ok": True}, image.pixels
        raise ValueError("unknown request %r" % op)

    @staticmethod
    def _find(items, id):
        for x in items:
            if x.id == id:
                return x
        raise ValueError("no effect or generator %r in this plugin" % id)


def serve(plugin, stdin, stdout):
    """Answers the app's requests from `stdin` until it closes."""
    while True:
        line = stdin.readline()
        if not line:
            return
        data = None
        try:
            request = json.loads(line)
            if request.get("op") == "apply":
                size = request["width"] * request["height"] * 4
                data = bytearray(stdin.read(size))
                if len(data) != size:
                    return
            answer, pixels = plugin.handle(request, data)
            if pixels is not None and len(pixels) != request["width"] * request["height"] * 4:
                raise ValueError("the image changed size")
        except Exception as e:  # reported to the app, which shows it on the layer
            answer, pixels = {"ok": False, "error": str(e) or type(e).__name__}, None
        stdout.write(json.dumps(answer).encode() + b"\n")
        if pixels is not None:
            stdout.write(pixels)
        stdout.flush()


def main(path):
    """Runs the plugin file `path` and answers the app's requests."""
    stdin, stdout = sys.stdin.buffer, sys.stdout.buffer
    # Anything printed goes to stderr, never into the answers.
    sys.stdout = sys.stderr
    sys.modules["artwork_plugin_sdk"] = sys.modules[__name__]
    plugin = runpy.run_path(path, run_name="artwork_plugin_sdk_main").get("plugin")
    if not isinstance(plugin, Plugin):
        raise SystemExit("%s defines no plugin = Plugin(...)" % path)
    serve(plugin, stdin, stdout)
