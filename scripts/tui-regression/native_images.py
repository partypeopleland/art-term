"""Inspect actual Sixel output without pretending the text emulator paints pixels."""
import hashlib
import re
from dataclasses import dataclass


@dataclass
class Raster:
    width: int
    height: int
    pixels: bytes

    def summary(self):
        return {"width": self.width, "height": self.height,
                "rgb_sha256": hashlib.sha256(self.pixels).hexdigest(),
                "colors": len({self.pixels[i:i + 3] for i in range(0, len(self.pixels), 3)})}


def decode_sixel(payload):
    assert re.match(rb"\x1bP[0-9;]*q", payload), "Unexpected terminal DCS"
    assert payload.endswith(b"\x1b\\")
    content = re.sub(rb"^\x1bP[0-9;]*q", b"", payload)[:-2].decode("ascii")
    raster = re.search(r'"(\d+);(\d+);(\d+);(\d+)', content)
    assert raster, "Sixel payload has no raster attributes"
    width, height = int(raster[3]), int(raster[4])
    assert 0 < width <= 10240 and 0 < height <= 10240 and width * height <= 4_000_000
    pixels = bytearray(width * height * 3)
    palette, selected, x, y, index = {}, 0, 0, 0, 0
    while index < len(content):
        char = content[index]
        if char == '"':
            match = re.match(r'"[0-9;]+', content[index:])
            index += len(match[0])
            continue
        if char == "#":
            match = re.match(r"#(\d+)(?:;2;(\d+);(\d+);(\d+))?", content[index:])
            assert match, "Unsupported Sixel palette encoding"
            selected = int(match[1])
            if match[2] is not None:
                assert all(0 <= int(match[n]) <= 100 for n in (2, 3, 4))
                palette[selected] = bytes(round(int(match[n]) * 255 / 100) for n in (2, 3, 4))
            index += len(match[0])
            continue
        if char == "$": x = 0
        elif char == "-": x, y = 0, y + 6
        elif char == "!" or "?" <= char <= "~":
            repeat = 1
            if char == "!":
                match = re.match(r"!(\d+)([?-~])", content[index:])
                assert match
                repeat, char = int(match[1]), match[2]
                index += len(match[0]) - 1
            bits = ord(char) - 63
            assert repeat <= width and x + repeat <= width, "Sixel run exceeds raster width"
            for bit in range(6):
                if bits & (1 << bit):
                    assert y + bit < height, "Sixel pixels exceed raster height"
                    assert selected in palette, "Sixel uses undefined palette entry"
                    color = palette[selected]
                    offset = ((y + bit) * width + x) * 3
                    pixels[offset:offset + repeat * 3] = color * repeat
            x += repeat
        else: assert char in "\r\n", ("Unknown Sixel token", char)
        index += 1
    return Raster(width, height, bytes(pixels))


class NativeImages:
    def __init__(self, screen, stream, decoder, cell_pixels):
        self.screen, self.stream, self.decoder = screen, stream, decoder
        self.cell_width, self.cell_height = cell_pixels
        self.pending = bytearray()
        self.payloads = []
        self.surfaces = {}

    def clear(self):
        self.surfaces.clear()

    def feed(self, data):
        self.pending.extend(data)
        while self.pending:
            start = self.pending.find(b"\x1bP")
            if start < 0:
                count = len(self.pending) - int(self.pending.endswith(b"\x1b"))
                self.stream.feed(self.decoder.decode(bytes(self.pending[:count])))
                del self.pending[:count]
                return
            if start:
                self.stream.feed(self.decoder.decode(bytes(self.pending[:start])))
                del self.pending[:start]
            end = self.pending.find(b"\x1b\\", 2)
            if end < 0: return
            payload = bytes(self.pending[:end + 2])
            raster = decode_sixel(payload)
            self.payloads.append(payload)
            self.surfaces[(self.screen.cursor.x, self.screen.cursor.y)] = raster
            del self.pending[:end + 2]

    def cells(self, color):
        expected = bytes.fromhex(color)
        cells = set()
        for (left, top), raster in self.surfaces.items():
            for row in range(raster.height):
                y = top + row // self.cell_height
                if not 0 <= y < self.screen.lines: continue
                for column in range(raster.width):
                    x = left + column // self.cell_width
                    if (x, y) in cells or not 0 <= x < self.screen.columns: continue
                    offset = (row * raster.width + column) * 3
                    pixel = raster.pixels[offset:offset + 3]
                    # Sixel palettes encode integer percentages and quantize RGB.
                    if all(abs(a - b) <= 4 for a, b in zip(pixel, expected)):
                        cells.add((x, y))
        return sorted(cells)

    def signature(self, region):
        left, top, right, bottom = region
        values = []
        for (x, y), raster in sorted(self.surfaces.items()):
            if x < right and y < bottom and x + (raster.width + self.cell_width - 1) // self.cell_width > left and y + (raster.height + self.cell_height - 1) // self.cell_height > top:
                values.append((x, y, raster.width, raster.height, hashlib.sha256(raster.pixels).hexdigest()))
        return tuple(values)
