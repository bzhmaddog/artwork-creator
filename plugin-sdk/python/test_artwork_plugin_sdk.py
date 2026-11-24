"""Tests of the Python plugin SDK (python3 -m unittest discover -s plugin-sdk/python)."""

import io
import json
import os
import subprocess
import sys
import tempfile
import unittest

import artwork_plugin_sdk as ap



class Fill(ap.Effect):
    id = "test.fill"
    name = "Fill"
    params = [ap.color_param("color", "Color", (10, 20, 30, 255)),
              ap.choice_param("fail", "Fail", [("no", "No"), ("yes", "Yes")], "no")]

    def padding(self, values):
        return (1, 2, 3, 4)

    def apply(self, image, values, scale):
        if values["fail"] == "yes":
            raise RuntimeError("asked to fail")
        image.pixels[:] = bytes(ap.color(values["color"])) * (image.width * image.height)


class Shrink(ap.Effect):
    id = "test.shrink"

    def apply(self, image, values, scale):
        del image.pixels[:4]


class Dot(ap.Generator):
    id = "test.dot"
    name = "Dot"

    def generate(self, image, values, px_per_mm):
        image.pixels[0:4] = b"\xff\x00\x00\xff"


PLUGIN = ap.Plugin("Test", "2.0", effects=[Fill(), Shrink()], generators=[Dot()])


def exchange(plugin, *requests):
    """Sends the requests (JSON, then bytes) to `serve` and splits its answers."""
    stdin = io.BytesIO(b"".join(json.dumps(r).encode() + b"\n" + data for r, data in requests))
    stdout = io.BytesIO()
    ap.serve(plugin, stdin, stdout)
    out = io.BytesIO(stdout.getvalue())
    answers = []
    for r, _ in requests:
        answer = json.loads(out.readline())
        pixels = None
        if answer["ok"] and r["op"] in ("apply", "generate"):
            pixels = out.read(r["width"] * r["height"] * 4)
        answers.append((answer, pixels))
    return answers


class SdkTest(unittest.TestCase):
    def test_params_are_the_documented_json(self):
        self.assertEqual(ap.float_param("r", "R", 0, 5, 1)["decimals"], 1)
        self.assertEqual(ap.int_param("n", "N", 1, 9, 3, unit=" x")["type"], "int")
        self.assertEqual(ap.bool_param("b", "B", True),
                         {"id": "b", "label": "B", "type": "bool", "default": True})
        self.assertEqual(ap.color_param("c", "C", (255, 0, 0, 128))["default"], "#ff000080")
        self.assertEqual(ap.color_param("c", "C", "#00ff00")["default"], "#00ff00")
        self.assertEqual(ap.choice_param("m", "M", [("a", "A")], "a")["options"],
                         [{"id": "a", "label": "A"}])
        self.assertEqual(ap.color("#ff000080"), (255, 0, 0, 128))
        self.assertEqual(ap.color("#0000ff"), (0, 0, 255, 255))

    def test_roles_are_given_only_when_asked(self):
        self.assertEqual(ap.int_param("v", "V", 0, 99, 0, role="seed")["role"], "seed")
        self.assertEqual(ap.float_param("a", "A", 0, 9, 0, unit="°", role="angle")["role"], "angle")
        self.assertNotIn("role", ap.int_param("n", "N", 1, 9, 3))
        self.assertNotIn("role", ap.float_param("r", "R", 0, 5, 1))
        seed = ap.seed_param("s", "S")
        self.assertEqual((seed["type"], seed["role"], seed["min"], seed["max"]), ("int", "seed", 0, 2**31 - 1))

    def test_groups_are_given_only_when_asked(self):
        self.assertEqual(ap.bool_param("b", "B", True, group="Edges")["group"], "Edges")
        self.assertEqual(ap.choice_param("m", "M", [("a", "A")], "a", group="G")["group"], "G")
        self.assertEqual(ap.seed_param("s", "S", group="G")["group"], "G")
        self.assertNotIn("group", ap.color_param("c", "C", "#00ff00"))
        self.assertNotIn("group", ap.float_param("r", "R", 0, 5, 1, role="size"))

    def test_describe(self):
        [(answer, _)] = exchange(PLUGIN, ({"op": "describe"}, b""))
        self.assertTrue(answer["ok"])
        self.assertEqual((answer["name"], answer["version"]), ("Test", "2.0"))
        self.assertEqual([(e["id"], e["padding"]) for e in answer["effects"]],
                         [("test.fill", True), ("test.shrink", False)])
        self.assertEqual(answer["generators"][0]["id"], "test.dot")

    def test_calls(self):
        values = {"color": "#0a141eff", "fail": "no"}
        answers = exchange(
            PLUGIN,
            ({"op": "padding", "id": "test.fill", "values": values}, b""),
            ({"op": "apply", "id": "test.fill", "values": values, "width": 2, "height": 1,
              "scale": 1.0}, bytes(8)),
            ({"op": "generate", "id": "test.dot", "values": {}, "width": 2, "height": 1,
              "px_per_mm": 1.0}, b""),
        )
        self.assertEqual(answers[0][0]["padding"], [1.0, 2.0, 3.0, 4.0])
        self.assertEqual(answers[1][1], b"\x0a\x14\x1e\xff" * 2)
        self.assertEqual(answers[2][1], b"\xff\x00\x00\xff" + bytes(4))

    def test_errors_are_answered_and_serving_goes_on(self):
        answers = exchange(
            PLUGIN,
            ({"op": "apply", "id": "test.fill", "values": {"fail": "yes"}, "width": 1,
              "height": 1, "scale": 1.0}, bytes(4)),
            ({"op": "apply", "id": "test.shrink", "values": {}, "width": 1, "height": 1,
              "scale": 1.0}, bytes(4)),
            ({"op": "apply", "id": "nope", "values": {}, "width": 1, "height": 1,
              "scale": 1.0}, bytes(4)),
            ({"op": "what"}, b""),
            ({"op": "padding", "id": "test.shrink", "values": {}}, b""),
        )
        self.assertEqual(answers[0][0], {"ok": False, "error": "asked to fail"})
        self.assertEqual(answers[1][0]["error"], "the image changed size")
        self.assertIn("nope", answers[2][0]["error"])
        self.assertFalse(answers[3][0]["ok"])
        self.assertEqual(answers[4][0]["padding"], [0.0] * 4)

    def test_stops_on_missing_pixels(self):
        stdout = io.BytesIO()
        request = {"op": "apply", "id": "test.fill", "values": {}, "width": 2, "height": 2,
                   "scale": 1.0}
        ap.serve(PLUGIN, io.BytesIO(json.dumps(request).encode() + b"\n" + bytes(3)), stdout)
        self.assertEqual(stdout.getvalue(), b"")

    def test_base_classes(self):
        image = ap.Image(1, 1)
        self.assertIsNone(ap.Effect().padding({}))
        with self.assertRaises(NotImplementedError):
            ap.Effect().apply(image, {}, 1.0)
        with self.assertRaises(NotImplementedError):
            ap.Generator().generate(image, {}, 1.0)

    def test_main_runs_a_plugin_file_as_the_app_does(self):
        with open(ap.__file__, encoding="utf-8") as f:
            boot = f.read() + "\nmain(sys.argv[1])\n"
        request = json.dumps({"op": "describe"}).encode() + b"\n"
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "sample.py")
            with open(path, "w", encoding="utf-8") as f:
                f.write("from artwork_plugin_sdk import Plugin\nplugin = Plugin('Sample', '1')\n")
            out = subprocess.run([sys.executable, "-c", boot, path], input=request,
                                 capture_output=True, check=True).stdout
        self.assertEqual(json.loads(out)["name"], "Sample")
        bad = subprocess.run([sys.executable, "-c", boot, os.devnull], input=b"",
                             capture_output=True)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn(b"defines no plugin", bad.stderr)


if __name__ == "__main__":
    unittest.main()
