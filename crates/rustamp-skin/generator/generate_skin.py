#!/usr/bin/env python3
"""
Procedural generator for the Rustamp Rust-themed built-in skin and lookalike fallback.

Clean-room design:
- Draws complete dark metallic chassis, buttons, displays, sliders, and fonts from scratch.
- Purely procedural: no external images, fonts, or assets.
- Produces all 32 classic skin entries (18 BMPs, 11 CURs, 3 TXTs) plus nums_ex.bmp at pixel-exact spec dimensions.
- Adheres strictly to specs/S01-window-geometry.md and specs/S17-skin-loading-quirks.md.
"""

import os
import struct
import math

# Output directory for the Rust-themed default skin assets
RUST_THEME_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "rust-theme")
LOOKALIKE_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "lookalike")

def create_bmp_24(width, height, get_pixel):
    """
    Creates uncompressed 24-bit Windows BMP bytes (bottom-up).
    get_pixel(x, y) returns (r, g, b) where 0 <= x < width, 0 <= y < height, top-down.
    """
    row_bytes = width * 3
    padding = (4 - (row_bytes % 4)) % 4
    image_size = (row_bytes + padding) * height
    file_size = 54 + image_size

    header = struct.pack(
        '<2sIHHI IiiHHIIiiII',
        b'BM',
        file_size,
        0, 0,
        54,
        40,
        width,
        height,
        1,
        24,
        0,
        image_size,
        2835, 2835,
        0, 0
    )

    data = bytearray(image_size)
    offset = 0
    pad = bytes(padding)
    for y in reversed(range(height)):
        for x in range(width):
            r, g, b = get_pixel(x, y)
            data[offset] = b & 0xFF
            data[offset + 1] = g & 0xFF
            data[offset + 2] = r & 0xFF
            offset += 3
        if padding:
            data[offset:offset + padding] = pad
            offset += padding

    return header + bytes(data)

def create_cur_32(width, height, hotspot_x, hotspot_y, get_rgba):
    """
    Creates a valid Windows .cur file (32-bit RGBA DIB + 1-bit AND mask).
    get_rgba(x, y) returns (r, g, b, a) where 0 <= x < width, 0 <= y < height, top-down.
    """
    and_mask_row_bytes = (width + 31) // 32 * 4
    and_mask_size = and_mask_row_bytes * height
    xor_size = width * height * 4
    dib_size = 40 + xor_size + and_mask_size

    header = struct.pack('<HHH', 0, 2, 1) # 2 = CURSOR
    dir_entry = struct.pack('<BBBBHHII', width, height, 0, 0, hotspot_x, hotspot_y, dib_size, 22)
    info_header = struct.pack('<IiiHHIIiiII', 40, width, height * 2, 1, 32, 0, xor_size, 0, 0, 0, 0)

    xor_data = bytearray(xor_size)
    and_mask = bytearray(and_mask_size)

    xor_idx = 0
    for y in reversed(range(height)):
        and_row = 0
        for x in range(width):
            r, g, b, a = get_rgba(x, y)
            xor_data[xor_idx:xor_idx+4] = bytes([b & 0xFF, g & 0xFF, r & 0xFF, a & 0xFF])
            xor_idx += 4
            if a == 0:
                and_row |= (1 << (31 - x))
        and_idx = (height - 1 - y) * and_mask_row_bytes
        struct.pack_into('>I', and_mask, and_idx, and_row)

    return header + dir_entry + info_header + bytes(xor_data) + bytes(and_mask)

class Canvas:
    def __init__(self, width, height, bg_color=(23, 24, 28)):
        self.width = width
        self.height = height
        self.pixels = [[bg_color for _ in range(width)] for _ in range(height)]

    def set(self, x, y, color):
        if 0 <= x < self.width and 0 <= y < self.height:
            self.pixels[y][x] = color

    def get(self, x, y):
        if 0 <= x < self.width and 0 <= y < self.height:
            return self.pixels[y][x]
        return (0, 0, 0)

    def fill_rect(self, x, y, w, h, color):
        for cy in range(max(0, y), min(self.height, y + h)):
            row = self.pixels[cy]
            for cx in range(max(0, x), min(self.width, x + w)):
                row[cx] = color

    def hline(self, x, y, w, color):
        self.fill_rect(x, y, w, 1, color)

    def vline(self, x, y, h, color):
        self.fill_rect(x, y, 1, h, color)

    def frame_rect(self, x, y, w, h, color):
        self.hline(x, y, w, color)
        self.hline(x, y + h - 1, w, color)
        self.vline(x, y, h, color)
        self.vline(x + w - 1, y, h, color)

    def bevel_raised(self, x, y, w, h, light=(62, 66, 73), shadow=(13, 14, 16)):
        self.hline(x, y, w, light)
        self.vline(x, y, h, light)
        self.hline(x, y + h - 1, w, shadow)
        self.vline(x + w - 1, y, h, shadow)

    def bevel_sunken(self, x, y, w, h, shadow=(10, 11, 13), light=(50, 54, 60)):
        self.hline(x, y, w, shadow)
        self.vline(x, y, h, shadow)
        self.hline(x, y + h - 1, w, light)
        self.vline(x + w - 1, y, h, light)

    def texture_metal(self, x, y, w, h, base_color=(23, 24, 28)):
        for cy in range(y, y + h):
            if cy < 0 or cy >= self.height:
                continue
            delta = (cy % 2) * 3 - 1
            r = max(0, min(255, base_color[0] + delta))
            g = max(0, min(255, base_color[1] + delta))
            b = max(0, min(255, base_color[2] + delta))
            for cx in range(x, x + w):
                if 0 <= cx < self.width:
                    self.pixels[cy][cx] = (r, g, b)

    def draw_gear(self, cx, cy, r_outer, r_inner, r_hole, teeth=8, color=(140, 59, 24), hole_color=(11, 12, 14)):
        """Draws a procedurally calculated gear / cog silhouette."""
        for dy in range(-r_outer, r_outer + 1):
            for dx in range(-r_outer, r_outer + 1):
                dist = math.sqrt(dx * dx + dy * dy)
                if dist > r_outer:
                    continue
                if dist < r_hole:
                    self.set(cx + dx, cy + dy, hole_color)
                    continue
                angle = math.atan2(dy, dx) % (2 * math.pi)
                tooth_angle = (2 * math.pi) / teeth
                phase = (angle % tooth_angle) / tooth_angle
                if dist <= r_inner or phase < 0.5:
                    self.set(cx + dx, cy + dy, color)

    def to_bmp(self):
        return create_bmp_24(self.width, self.height, lambda x, y: self.pixels[y][x])


# 5x6 bitmap font definition for text.bmp
FONT_5X6 = {
    'a': ["01110", "10001", "11111", "10001", "10001", "00000"],
    'b': ["11110", "10001", "11110", "10001", "11110", "00000"],
    'c': ["01111", "10000", "10000", "10000", "01111", "00000"],
    'd': ["11110", "10001", "10001", "10001", "11110", "00000"],
    'e': ["11111", "10000", "11110", "10000", "11111", "00000"],
    'f': ["11111", "10000", "11110", "10000", "10000", "00000"],
    'g': ["01111", "10000", "10111", "10001", "01111", "00000"],
    'h': ["10001", "10001", "11111", "10001", "10001", "00000"],
    'i': ["11111", "00100", "00100", "00100", "11111", "00000"],
    'j': ["00001", "00001", "00001", "10001", "01110", "00000"],
    'k': ["10001", "10010", "11100", "10010", "10001", "00000"],
    'l': ["10000", "10000", "10000", "10000", "11111", "00000"],
    'm': ["10001", "11011", "10101", "10001", "10001", "00000"],
    'n': ["10001", "11001", "10101", "10011", "10001", "00000"],
    'o': ["01110", "10001", "10001", "10001", "01110", "00000"],
    'p': ["11110", "10001", "11110", "10000", "10000", "00000"],
    'q': ["01110", "10001", "10001", "10011", "01111", "00000"],
    'r': ["11110", "10001", "11110", "10010", "10001", "00000"],
    's': ["01111", "10000", "01110", "00001", "11110", "00000"],
    't': ["11111", "00100", "00100", "00100", "00100", "00000"],
    'u': ["10001", "10001", "10001", "10001", "01110", "00000"],
    'v': ["10001", "10001", "10001", "01010", "00100", "00000"],
    'w': ["10001", "10001", "10101", "11011", "10001", "00000"],
    'x': ["10001", "01010", "00100", "01010", "10001", "00000"],
    'y': ["10001", "01010", "00100", "00100", "00100", "00000"],
    'z': ["11111", "00010", "00100", "01000", "11111", "00000"],
    '"': ["01010", "01010", "00000", "00000", "00000", "00000"],
    '@': ["01110", "10011", "10101", "10000", "01111", "00000"],
    ' ': ["00000", "00000", "00000", "00000", "00000", "00000"],

    '0': ["01110", "10011", "10101", "11001", "01110", "00000"],
    '1': ["00100", "01100", "00100", "00100", "01110", "00000"],
    '2': ["01110", "10001", "00010", "01100", "11111", "00000"],
    '3': ["11110", "00001", "01110", "00001", "11110", "00000"],
    '4': ["10010", "10010", "11111", "00010", "00010", "00000"],
    '5': ["11111", "10000", "11110", "00001", "11110", "00000"],
    '6': ["01110", "10000", "11110", "10001", "01110", "00000"],
    '7': ["11111", "00010", "00100", "01000", "01000", "00000"],
    '8': ["01110", "10001", "01110", "10001", "01110", "00000"],
    '9': ["01110", "10001", "01111", "00001", "01110", "00000"],
    '…': ["00000", "00000", "00000", "00000", "10101", "00000"],
    '.': ["00000", "00000", "00000", "00000", "01100", "00000"],
    ':': ["00000", "01100", "00000", "01100", "00000", "00000"],
    '(': ["00010", "00100", "00100", "00100", "00010", "00000"],
    ')': ["01000", "00100", "00100", "00100", "01000", "00000"],
    '-': ["00000", "00000", "11111", "00000", "00000", "00000"],
    '\'': ["00100", "00100", "00000", "00000", "00000", "00000"],
    '!': ["00100", "00100", "00100", "00000", "00100", "00000"],
    '_': ["00000", "00000", "00000", "00000", "11111", "00000"],
    '+': ["00000", "00100", "01110", "00100", "00000", "00000"],
    '\\': ["10000", "01000", "00100", "00010", "00001", "00000"],
    '/': ["00001", "00010", "00100", "01000", "10000", "00000"],
    '[': ["01110", "01000", "01000", "01000", "01110", "00000"],
    ']': ["01110", "00010", "00010", "00010", "01110", "00000"],
    '^': ["00100", "01010", "10001", "00000", "00000", "00000"],
    '&': ["01100", "10010", "01100", "10010", "01101", "00000"],
    '%': ["10001", "00010", "00100", "01000", "10001", "00000"],
    ',': ["00000", "00000", "00000", "00100", "00100", "01000"],
    '=': ["00000", "11111", "00000", "11111", "00000", "00000"],
    '$': ["00100", "01111", "00100", "11110", "00100", "00000"],
    '#': ["01010", "11111", "01010", "11111", "01010", "00000"],

    'Å': ["00100", "01110", "10001", "11111", "10001", "00000"],
    'Ö': ["10101", "01110", "10001", "10001", "01110", "00000"],
    'Ä': ["10101", "01110", "10001", "11111", "10001", "00000"],
    '?': ["01110", "10001", "00110", "00000", "00100", "00000"],
    '*': ["10101", "01110", "11111", "01110", "10101", "00000"],
}

def draw_text_glyph(canvas, char, x, y, fg=(240, 242, 245), bg=(0, 0, 0)):
    grid = FONT_5X6.get(char.lower())
    if not grid:
        grid = FONT_5X6.get('?')
    for cy in range(6):
        line = grid[cy]
        for cx in range(5):
            canvas.set(x + cx, y + cy, fg if line[cx] == '1' else bg)

def draw_string(canvas, text, x, y, fg=(240, 242, 245), bg=(0, 0, 0)):
    cur_x = x
    for ch in text:
        draw_text_glyph(canvas, ch, cur_x, y, fg, bg)
        cur_x += 5

# 7-Segment digit patterns for 9x13 cells
SEGMENTS_7 = {
    '0': {'a', 'b', 'c', 'd', 'e', 'f'},
    '1': {'b', 'c'},
    '2': {'a', 'b', 'd', 'e', 'g'},
    '3': {'a', 'b', 'c', 'd', 'g'},
    '4': {'b', 'c', 'f', 'g'},
    '5': {'a', 'c', 'd', 'f', 'g'},
    '6': {'a', 'c', 'd', 'e', 'f', 'g'},
    '7': {'a', 'b', 'c'},
    '8': {'a', 'b', 'c', 'd', 'e', 'f', 'g'},
    '9': {'a', 'b', 'c', 'd', 'f', 'g'},
    ' ': set(),
}

def draw_7segment_cell(canvas, char, x, y, lit_color=(255, 122, 41), dim_color=(48, 22, 13), bg_color=(11, 12, 14)):
    canvas.fill_rect(x, y, 9, 13, bg_color)
    segs = SEGMENTS_7.get(char, set())

    def seg_a():
        col = lit_color if 'a' in segs else dim_color
        canvas.hline(x + 2, y + 1, 5, col)
    def seg_b():
        col = lit_color if 'b' in segs else dim_color
        canvas.vline(x + 7, y + 2, 4, col)
    def seg_c():
        col = lit_color if 'c' in segs else dim_color
        canvas.vline(x + 7, y + 7, 4, col)
    def seg_d():
        col = lit_color if 'd' in segs else dim_color
        canvas.hline(x + 2, y + 11, 5, col)
    def seg_e():
        col = lit_color if 'e' in segs else dim_color
        canvas.vline(x + 1, y + 7, 4, col)
    def seg_f():
        col = lit_color if 'f' in segs else dim_color
        canvas.vline(x + 1, y + 2, 4, col)
    def seg_g():
        col = lit_color if 'g' in segs else dim_color
        canvas.hline(x + 2, y + 6, 5, col)

    seg_a()
    seg_b()
    seg_c()
    seg_d()
    seg_e()
    seg_f()
    seg_g()


# --- Rust Theme BMP Generators ---

def generate_rust_main():
    """Generates main.bmp (275 x 116). S01 §5, line 680."""
    c = Canvas(275, 116, bg_color=(23, 24, 28))
    c.texture_metal(0, 0, 275, 116, base_color=(23, 24, 28))
    c.bevel_raised(0, 0, 275, 116, light=(62, 66, 73), shadow=(13, 14, 16))

    # Main display cutout: (10, 22) to (264, 68) -> 255 x 46
    c.fill_rect(10, 22, 255, 46, (11, 12, 14))
    c.bevel_sunken(10, 22, 255, 46, shadow=(8, 9, 10), light=(45, 48, 55))

    # Time display colon dots in Rust orange at (73, 29) and (73, 35)
    c.fill_rect(73, 29, 2, 2, (255, 107, 43))
    c.fill_rect(73, 35, 2, 2, (255, 107, 43))

    # Display indicator labels: "kbps" and "khz" in warm subtle rust
    draw_string(c, "kbps", 130, 43, fg=(110, 50, 28), bg=(11, 12, 14))
    draw_string(c, "khz", 175, 43, fg=(110, 50, 28), bg=(11, 12, 14))

    # Position bar groove at (16, 72, 248, 10)
    c.fill_rect(16, 72, 248, 10, (14, 15, 17))
    c.bevel_sunken(16, 72, 248, 10, shadow=(8, 8, 9), light=(45, 48, 55))

    # Volume slider groove at (107, 57, 68, 14)
    c.fill_rect(107, 57, 68, 14, (14, 15, 17))
    c.bevel_sunken(107, 57, 68, 14, shadow=(8, 8, 9), light=(45, 48, 55))

    # Balance slider groove at (177, 57, 38, 14)
    c.fill_rect(177, 57, 38, 14, (14, 15, 17))
    c.bevel_sunken(177, 57, 38, 14, shadow=(8, 8, 9), light=(45, 48, 55))

    # Transport button well at (15, 87, 143, 20)
    c.bevel_sunken(15, 87, 143, 20, shadow=(12, 13, 15), light=(50, 54, 60))

    return c.to_bmp()

def generate_rust_titlebar():
    """Generates titlebar.bmp (344 x 87). S01 §5, line 681; fixture exact."""
    c = Canvas(344, 87, bg_color=(20, 21, 24))

    # 1. Focused titlebar at (27, 0, 275, 14)
    # Deep burnished rust to dark graphite gradient
    for x in range(27, 302):
        t = (x - 27) / 275.0
        r = int(66 * (1.0 - t * 0.7))
        g = int(26 * (1.0 - t * 0.5))
        b = int(14 * (1.0 - t * 0.3))
        for y in range(0, 14):
            line_hl = 12 if (y == 1 or y == 12) else 0
            c.set(x, y, (min(255, r + line_hl), min(255, g + line_hl), min(255, b + line_hl)))
    c.bevel_raised(27, 0, 275, 14, light=(140, 60, 25), shadow=(18, 10, 8))
    # Title lettering "RUSTAMP"
    draw_string(c, "rustamp", 80, 4, fg=(255, 200, 160), bg=(45, 20, 12))

    # 2. Unfocused titlebar at (27, 15, 275, 14)
    for x in range(27, 302):
        t = (x - 27) / 275.0
        r = int(36 - 8 * t)
        g = int(38 - 8 * t)
        b = int(42 - 8 * t)
        for y in range(15, 29):
            line_hl = 8 if (y == 16 or y == 27) else 0
            c.set(x, y, (r + line_hl, g + line_hl, b + line_hl))
    c.bevel_raised(27, 15, 275, 14, light=(55, 58, 65), shadow=(12, 13, 15))
    draw_string(c, "rustamp", 80, 19, fg=(120, 125, 135), bg=(32, 34, 38))

    # 3. Shade titlebars (focused at (27, 29, 275, 14), unfocused at (27, 42, 275, 14))
    for y in range(14):
        for x in range(275):
            c.set(27 + x, 29 + y, c.get(27 + x, y))
            c.set(27 + x, 42 + y, c.get(27 + x, 15 + y))

    # 4. Easter egg titlebars at (27, 57) and (27, 72)
    for y in range(14):
        for x in range(275):
            c.set(27 + x, 57 + y, c.get(27 + x, y))
            c.set(27 + x, 72 + y, c.get(27 + x, 15 + y))

    # Window buttons within title bars
    for base_y in [0, 15, 29, 42, 57, 72]:
        # Options button at x=27+3
        c.fill_rect(27 + 3, base_y + 3, 9, 8, (45, 48, 55))
        c.bevel_raised(27 + 3, base_y + 3, 9, 8, light=(80, 85, 95), shadow=(12, 13, 15))
        c.hline(27 + 5, base_y + 5, 5, (220, 225, 235))
        c.hline(27 + 5, base_y + 7, 5, (220, 225, 235))
        c.hline(27 + 5, base_y + 9, 5, (220, 225, 235))

        # Minimize at 27+244
        c.fill_rect(27 + 244, base_y + 3, 9, 8, (45, 48, 55))
        c.bevel_raised(27 + 244, base_y + 3, 9, 8, light=(80, 85, 95), shadow=(12, 13, 15))
        c.hline(27 + 246, base_y + 8, 5, (240, 242, 248))

        # Shade at 27+254
        c.fill_rect(27 + 254, base_y + 3, 9, 8, (45, 48, 55))
        c.bevel_raised(27 + 254, base_y + 3, 9, 8, light=(80, 85, 95), shadow=(12, 13, 15))
        c.frame_rect(27 + 256, base_y + 5, 5, 4, (240, 242, 248))

        # Close at 27+264
        c.fill_rect(27 + 264, base_y + 3, 9, 8, (45, 48, 55))
        c.bevel_raised(27 + 264, base_y + 3, 9, 8, light=(80, 85, 95), shadow=(12, 13, 15))
        for d in range(5):
            c.set(27 + 266 + d, base_y + 5 + d, (255, 107, 43))
            c.set(27 + 270 - d, base_y + 5 + d, (255, 107, 43))

    # Standalone window buttons at (0, 0) .. (26, 36)
    for bx, by, sym in [(0, 0, 'opt'), (9, 0, 'min'), (0, 18, 'shade'), (18, 0, 'close')]:
        c.fill_rect(bx, by, 9, 9, (45, 48, 55))
        c.bevel_raised(bx, by, 9, 9, light=(75, 80, 90), shadow=(12, 13, 15))
        dep_x, dep_y = (bx, by + 9) if by == 0 else (bx + 9, by)
        c.fill_rect(dep_x, dep_y, 9, 9, (32, 34, 40))
        c.bevel_sunken(dep_x, dep_y, 9, 9, shadow=(10, 11, 13), light=(65, 70, 80))

    # Clutter bar background at (304, 0, 8, 43)
    c.fill_rect(304, 0, 8, 43, (20, 21, 25))
    c.bevel_sunken(304, 0, 8, 43, shadow=(10, 11, 13), light=(45, 48, 55))
    c.fill_rect(312, 0, 8, 43, (17, 18, 21))
    c.bevel_sunken(312, 0, 8, 43, shadow=(8, 9, 10), light=(38, 40, 46))

    # Clutter bar lit buttons (O, A, I, D, V) at (308..340, 47..78) in glowing Rust orange
    lit = (255, 107, 43)
    # O (308, 47, 4, 6)
    c.fill_rect(308, 47, 4, 6, (11, 12, 14))
    c.frame_rect(308, 47, 4, 6, lit)
    # A (316, 55, 4, 6)
    c.fill_rect(316, 55, 4, 6, (11, 12, 14))
    c.hline(317, 55, 2, lit)
    c.vline(316, 56, 5, lit)
    c.vline(319, 56, 5, lit)
    c.hline(316, 58, 4, lit)
    # I (324, 63, 3, 6)
    c.fill_rect(324, 63, 3, 6, (11, 12, 14))
    c.vline(325, 63, 6, lit)
    # D (332, 71, 4, 6)
    c.fill_rect(332, 71, 4, 6, (11, 12, 14))
    c.vline(332, 71, 6, lit)
    c.hline(332, 71, 3, lit)
    c.hline(332, 76, 3, lit)
    c.vline(335, 72, 4, lit)
    # V (340, 78, 4, 6)
    c.fill_rect(340, 78, 4, 6, (11, 12, 14))
    c.vline(340, 78, 4, lit)
    c.vline(343, 78, 4, lit)
    c.set(341, 82, lit)
    c.set(342, 82, lit)

    # Shade position thumb at (0, 36)
    c.fill_rect(0, 36, 17, 7, (14, 15, 17))
    c.fill_rect(17, 36, 9, 7, (55, 60, 70))
    c.bevel_raised(17, 36, 9, 7, light=(90, 100, 115), shadow=(18, 20, 24))

    return c.to_bmp()

def generate_rust_cbuttons():
    """Generates cbuttons.bmp (136 x 36). S01 §5, line 682."""
    c = Canvas(136, 36, bg_color=(20, 21, 24))

    buttons = [
        (0, 0, 22, 18, 'prev'),
        (23, 0, 22, 18, 'play'),
        (46, 0, 22, 18, 'pause'),
        (69, 0, 22, 18, 'stop'),
        (92, 0, 22, 18, 'next'),
        (114, 0, 22, 16, 'eject'),
    ]

    for x, y, w, h, kind in buttons:
        # Resting state (y=0)
        c.fill_rect(x, y, w, h, (40, 42, 48))
        c.bevel_raised(x, y, w, h, light=(75, 80, 90), shadow=(12, 13, 16))
        # Pressed state (y=18, eject y=16)
        py = 18 if kind != 'eject' else 16
        c.fill_rect(x, py, w, h, (28, 30, 35))
        c.bevel_sunken(x, py, w, h, shadow=(10, 11, 13), light=(60, 65, 75))

        for is_pressed in [False, True]:
            ox = x + (1 if is_pressed else 0)
            oy = (py if is_pressed else y) + (1 if is_pressed else 0)

            # Play button has signature Rust orange icon; others off-white
            if kind == 'play':
                icon_col = (255, 107, 43) if not is_pressed else (210, 70, 20)
            else:
                icon_col = (240, 242, 248) if not is_pressed else (170, 175, 185)

            if kind == 'prev':
                c.vline(ox + 4, oy + 4, 10, icon_col)
                for row in range(5):
                    c.vline(ox + 9 - row, oy + 4 + row, (5 - row) * 2, icon_col)
                for row in range(5):
                    c.vline(ox + 15 - row, oy + 4 + row, (5 - row) * 2, icon_col)
            elif kind == 'play':
                for col in range(9):
                    c.vline(ox + 6 + col, oy + 4 + col // 2, 10 - col, icon_col)
            elif kind == 'pause':
                c.fill_rect(ox + 6, oy + 4, 3, 10, icon_col)
                c.fill_rect(ox + 12, oy + 4, 3, 10, icon_col)
            elif kind == 'stop':
                c.fill_rect(ox + 6, oy + 4, 10, 10, icon_col)
            elif kind == 'next':
                for row in range(5):
                    c.vline(ox + 5 + row, oy + 4 + row, (5 - row) * 2, icon_col)
                for row in range(5):
                    c.vline(ox + 11 + row, oy + 4 + row, (5 - row) * 2, icon_col)
                c.vline(ox + 16, oy + 4, 10, icon_col)
            elif kind == 'eject':
                for row in range(5):
                    c.hline(ox + 10 - row, oy + 4 + row, row * 2 + 1, icon_col)
                c.hline(ox + 6, oy + 11, 10, icon_col)

    return c.to_bmp()

def generate_rust_shufrep():
    """Generates shufrep.bmp (92 x 85). S01 §5, line 683."""
    c = Canvas(92, 85, bg_color=(20, 21, 24))

    for state, sy in enumerate([0, 15, 30, 45]):
        is_dep = (state % 2 == 1)
        is_sel = (state >= 2)
        bg = (28, 30, 35) if is_dep else (40, 42, 48)

        # Repeat (0, 0, 28, 15)
        c.fill_rect(0, sy, 28, 15, bg)
        if is_dep:
            c.bevel_sunken(0, sy, 28, 15, shadow=(10, 11, 13), light=(60, 65, 75))
        else:
            c.bevel_raised(0, sy, 28, 15, light=(75, 80, 90), shadow=(12, 13, 16))

        # Shuffle (28, 0, 47, 15)
        c.fill_rect(28, sy, 47, 15, bg)
        if is_dep:
            c.bevel_sunken(28, sy, 47, 15, shadow=(10, 11, 13), light=(60, 65, 75))
        else:
            c.bevel_raised(28, sy, 47, 15, light=(75, 80, 90), shadow=(12, 13, 16))

        # Glowing Rust orange LED when selected
        led_col = (255, 107, 43) if is_sel else (45, 20, 12)
        c.fill_rect(3, sy + 6, 3, 3, led_col)
        draw_string(c, "rep", 8, sy + 4, fg=(240, 242, 248), bg=bg)

        c.fill_rect(31, sy + 6, 3, 3, led_col)
        draw_string(c, "shuffle", 36, sy + 4, fg=(240, 242, 248), bg=bg)

    # EQ and PL buttons
    for bx, by, kind, is_dep, is_sel in [
        (0, 61, 'eq', False, False),
        (0, 73, 'eq', False, True),
        (46, 61, 'eq', True, False),
        (46, 73, 'eq', True, True),
        (23, 61, 'pl', False, False),
        (23, 73, 'pl', False, True),
        (69, 61, 'pl', True, False),
        (69, 73, 'pl', True, True),
    ]:
        bg = (28, 30, 35) if is_dep else (40, 42, 48)
        c.fill_rect(bx, by, 23, 12, bg)
        if is_dep:
            c.bevel_sunken(bx, by, 23, 12, shadow=(10, 11, 13), light=(60, 65, 75))
        else:
            c.bevel_raised(bx, by, 23, 12, light=(75, 80, 90), shadow=(12, 13, 16))

        fg = (255, 107, 43) if is_sel else (170, 175, 185)
        text = "eq" if kind == 'eq' else "pl"
        draw_string(c, text, bx + 6, by + 3, fg=fg, bg=bg)

    return c.to_bmp()

def generate_rust_posbar():
    """Generates posbar.bmp (307 x 10). S01 §5, line 684."""
    c = Canvas(307, 10, bg_color=(14, 15, 17))

    # Track at (0, 0, 248, 10)
    c.fill_rect(0, 0, 248, 10, (16, 17, 20))
    c.bevel_sunken(0, 0, 248, 10, shadow=(8, 9, 10), light=(45, 48, 55))
    c.hline(2, 4, 244, (9, 10, 12))
    c.hline(2, 5, 244, (28, 30, 35))

    # Resting thumb at (246, 0, 29, 10)
    c.fill_rect(246, 0, 29, 10, (55, 58, 66))
    c.bevel_raised(246, 0, 29, 10, light=(95, 100, 115), shadow=(18, 20, 24))
    # Grip lines in glowing Rust orange
    for gx in [254, 258, 262, 266]:
        c.vline(gx, 2, 6, (255, 107, 43))
        c.vline(gx + 1, 2, 6, (28, 30, 35))

    # Dragged thumb at (278, 0, 29, 10)
    c.fill_rect(278, 0, 29, 10, (45, 48, 56))
    c.bevel_sunken(278, 0, 29, 10, shadow=(14, 15, 18), light=(80, 85, 95))
    for gx in [286, 290, 294, 298]:
        c.vline(gx, 2, 6, (210, 70, 20))
        c.vline(gx + 1, 2, 6, (95, 100, 115))

    return c.to_bmp()

def generate_rust_volume():
    """Generates volume.bmp (68 x 433). S01 §5, line 685."""
    c = Canvas(68, 433, bg_color=(14, 15, 17))

    for frame in range(28):
        fy = frame * 15
        c.fill_rect(0, fy, 68, 15, (18, 19, 22))
        c.bevel_sunken(0, fy, 68, 14, shadow=(8, 9, 10), light=(45, 48, 55))

        num_bars = frame
        for b in range(27):
            bx = 8 + b * 2
            bh = 2 + int((b / 27.0) * 6)
            by = fy + 10 - bh
            if b <= num_bars:
                # Fire/ember gradient: golden amber -> fiery Rust orange -> searing red
                if b < 16:
                    col = (230, 160, 20)
                elif b < 22:
                    col = (255, 107, 43)
                else:
                    col = (229, 57, 28)
            else:
                col = (42, 22, 16) # Unlit ember bar
            c.fill_rect(bx, by, 1, bh, col)

    # Thumbs at y=420 (14 x 11): Selected at (0, 420), Resting at (15, 420)
    for tx, is_sel in [(15, False), (0, True)]:
        bg = (55, 58, 66) if not is_sel else (42, 45, 52)
        c.fill_rect(tx, 420, 14, 11, bg)
        if is_sel:
            c.bevel_sunken(tx, 420, 14, 11, shadow=(14, 15, 18), light=(85, 90, 100))
        else:
            c.bevel_raised(tx, 420, 14, 11, light=(95, 100, 115), shadow=(18, 20, 24))
        notch_col = (255, 107, 43) if is_sel else (25, 28, 32)
        c.vline(tx + 6, 422, 7, notch_col)
        c.vline(tx + 7, 422, 7, (90, 95, 105))

    return c.to_bmp()

def generate_rust_balance():
    """Generates balance.bmp (68 x 433). S01 §5, line 686; fixture exact."""
    c = Canvas(68, 433, bg_color=(14, 15, 17))

    for frame in range(28):
        fy = frame * 15
        c.fill_rect(9, fy, 38, 14, (18, 19, 22))
        c.bevel_sunken(9, fy, 38, 14, shadow=(8, 9, 10), light=(45, 48, 55))
        c.vline(28, fy + 2, 10, (38, 40, 46))

        pan_offset = frame - 14
        bar_x = 28 + int((pan_offset / 14.0) * 14)
        c.fill_rect(max(11, min(43, bar_x - 1)), fy + 4, 3, 6, (255, 107, 43))

    for tx, is_sel in [(15, False), (0, True)]:
        bg = (55, 58, 66) if not is_sel else (42, 45, 52)
        c.fill_rect(tx, 420, 14, 11, bg)
        if is_sel:
            c.bevel_sunken(tx, 420, 14, 11, shadow=(14, 15, 18), light=(85, 90, 100))
        else:
            c.bevel_raised(tx, 420, 14, 11, light=(95, 100, 115), shadow=(18, 20, 24))
        notch_col = (255, 107, 43) if is_sel else (25, 28, 32)
        c.vline(tx + 6, 422, 7, notch_col)
        c.vline(tx + 7, 422, 7, (90, 95, 105))

    return c.to_bmp()

def generate_rust_monoster():
    """Generates monoster.bmp (58 x 24). S01 §5, line 687; fixture exact."""
    c = Canvas(58, 24, bg_color=(11, 12, 14))

    # Stereo: Lit at (0, 0, 29, 12), Dim at (0, 12, 29, 12)
    c.fill_rect(0, 0, 29, 12, (11, 12, 14))
    draw_string(c, "stereo", 0, 3, fg=(255, 107, 43), bg=(11, 12, 14))
    c.fill_rect(0, 12, 29, 12, (11, 12, 14))
    draw_string(c, "stereo", 0, 15, fg=(48, 22, 13), bg=(11, 12, 14))

    # Mono: Lit at (29, 0, 27, 12), Dim at (29, 12, 27, 12)
    c.fill_rect(29, 0, 27, 12, (11, 12, 14))
    draw_string(c, "mono", 32, 3, fg=(255, 107, 43), bg=(11, 12, 14))
    c.fill_rect(29, 12, 27, 12, (11, 12, 14))
    draw_string(c, "mono", 32, 15, fg=(48, 22, 13), bg=(11, 12, 14))

    return c.to_bmp()

def generate_rust_playpaus():
    """Generates playpaus.bmp (42 x 9). S01 §5, line 688."""
    c = Canvas(42, 9, bg_color=(11, 12, 14))

    # Playing (0, 0, 9, 9): Glowing Rust orange triangle
    for col in range(6):
        c.vline(2 + col, 2 + col // 2, 5 - (col // 2) * 2, (255, 107, 43))

    # Paused (9, 0, 9, 9): Warm amber bars
    c.fill_rect(11, 2, 2, 5, (255, 179, 71))
    c.fill_rect(15, 2, 2, 5, (255, 179, 71))

    # Stopped (18, 0, 9, 9): Red square
    c.fill_rect(20, 2, 5, 5, (211, 47, 47))

    # Working indicator at (36, 0, 3, 9): Bright flame highlight
    c.fill_rect(36, 0, 3, 9, (255, 160, 92))

    # Idle indicator at (39, 0, 3, 9): Dark graphite
    c.fill_rect(39, 0, 3, 9, (25, 27, 32))

    return c.to_bmp()

def generate_rust_numbers():
    """Generates numbers.bmp (99 x 13). S01 §5, line 689."""
    c = Canvas(99, 13, bg_color=(11, 12, 14))

    for digit in range(10):
        draw_7segment_cell(c, str(digit), digit * 9, 0)

    draw_7segment_cell(c, ' ', 90, 0)
    # Minus sign at (20, 6, 5, 1) solid orange
    c.fill_rect(20, 6, 5, 1, (255, 122, 41))
    # No-minus placeholder at (9, 6, 5, 1) dim
    c.fill_rect(9, 6, 5, 1, (48, 22, 13))

    return c.to_bmp()

def generate_rust_nums_ex():
    """Generates nums_ex.bmp (108 x 13). S01 §2.3."""
    c = Canvas(108, 13, bg_color=(11, 12, 14))

    for digit in range(10):
        draw_7segment_cell(c, str(digit), digit * 9, 0)

    draw_7segment_cell(c, ' ', 90, 0)
    draw_7segment_cell(c, ' ', 99, 0)
    c.fill_rect(99 + 2, 6, 5, 1, (255, 122, 41))

    return c.to_bmp()

def generate_rust_text():
    """Generates text.bmp (155 x 74). S01 §5, line 690."""
    c = Canvas(155, 74, bg_color=(0, 0, 0))

    row0 = list("abcdefghijklmnopqrstuvwxyz\"@  ")
    row1 = list("0123456789….:()-'!_+\\/[]^&%,=$#")
    row2 = ["Å", "Ö", "Ä", "?", "*"]

    # 4 distinct palette stripes for the Rust theme
    color_schemes = [
        ((240, 242, 245), (0, 0, 0)),       # y = 0: Off-white on black
        ((255, 122, 41), (20, 21, 24)),     # y = 18: Bright Rust orange on dark charcoal
        ((255, 179, 102), (0, 0, 0)),      # y = 38: Warm amber on black
        ((255, 255, 255), (0, 0, 0)),      # y = 56: Pure white on black
    ]

    for base_y, (fg, bg) in zip([0, 18, 38, 56], color_schemes):
        for col_idx, ch in enumerate(row0):
            if col_idx < 31:
                draw_text_glyph(c, ch, col_idx * 5, base_y + 0, fg, bg)
        for col_idx, ch in enumerate(row1):
            if col_idx < 31:
                draw_text_glyph(c, ch, col_idx * 5, base_y + 6, fg, bg)
        for col_idx, ch in enumerate(row2):
            if col_idx < 31:
                draw_text_glyph(c, ch, col_idx * 5, base_y + 12, fg, bg)

    return c.to_bmp()

def generate_rust_pledit():
    """Generates pledit.bmp (280 x 186). S01 §5, line 691."""
    c = Canvas(280, 186, bg_color=(14, 15, 17))

    for focused, base_y in [(True, 0), (False, 21)]:
        bg = (48, 22, 14) if focused else (32, 34, 38)
        fg = (255, 200, 160) if focused else (120, 125, 135)

        for bx, bw in [(0, 25), (26, 100), (127, 25), (153, 25)]:
            c.fill_rect(bx, base_y, bw, 20, bg)
            c.bevel_raised(bx, base_y, bw, 20, light=(90, 45, 25) if focused else (55, 58, 65), shadow=(15, 10, 8))

        draw_string(c, "playlist", 45, base_y + 7, fg=fg, bg=bg)

        # The window buttons in the right corner: shade at window x 254, close
        # at window x 264, both at y 3 (the corner starts at window x 250).
        for offset in (4, 14):
            c.fill_rect(153 + offset, base_y + 3, 9, 9, (45, 48, 55))
            c.bevel_raised(153 + offset, base_y + 3, 9, 9, light=(80, 85, 95), shadow=(12, 13, 15))
        c.hline(153 + 6, base_y + 8, 5, fg)
        for d in range(5):
            c.set(153 + 16 + d, base_y + 5 + d, (255, 107, 43))
            c.set(153 + 20 - d, base_y + 5 + d, (255, 107, 43))

    # The pressed window buttons: close, collapse, expand.
    for bx, by, symbol in [(52, 42, "close"), (62, 42, "shade"), (150, 42, "shade")]:
        c.fill_rect(bx, by, 9, 9, (24, 25, 29))
        c.bevel_sunken(bx, by, 9, 9, shadow=(8, 9, 10), light=(70, 74, 82))
        if symbol == "close":
            for d in range(5):
                c.set(bx + 2 + d, by + 2 + d, (255, 107, 43))
                c.set(bx + 6 - d, by + 2 + d, (255, 107, 43))
        else:
            c.hline(bx + 2, by + 5, 5, (255, 200, 160))

    # Left rail at (0, 42, 12, 29)
    c.fill_rect(0, 42, 12, 29, (28, 30, 35))
    c.bevel_raised(0, 42, 12, 29, light=(55, 58, 65), shadow=(10, 11, 13))

    # Right rail / scrollbar track at (31, 42, 20, 29)
    c.fill_rect(31, 42, 20, 29, (18, 19, 22))
    c.bevel_sunken(31, 42, 20, 29, shadow=(8, 9, 10), light=(45, 48, 55))

    # Scroll handle at (52, 53, 8, 18)
    c.fill_rect(52, 53, 8, 18, (50, 54, 62))
    c.bevel_raised(52, 53, 8, 18, light=(85, 90, 100), shadow=(18, 20, 24))
    c.fill_rect(61, 53, 8, 18, (42, 45, 52))
    c.bevel_sunken(61, 53, 8, 18, shadow=(14, 15, 18), light=(75, 80, 90))

    # Scroll arrows at (261, 72, 8, 10)
    c.fill_rect(261, 72, 8, 10, (40, 42, 48))
    c.bevel_raised(261, 72, 8, 10, light=(75, 80, 90), shadow=(12, 13, 16))
    c.set(264, 74, (255, 107, 43))
    c.hline(263, 75, 3, (255, 107, 43))
    c.hline(263, 78, 3, (255, 107, 43))
    c.set(264, 79, (255, 107, 43))

    # Bottom bar blocks
    c.fill_rect(0, 72, 125, 38, (28, 30, 35))
    c.bevel_raised(0, 72, 125, 38, light=(60, 64, 72), shadow=(12, 13, 15))
    c.fill_rect(126, 72, 150, 38, (28, 30, 35))
    c.bevel_raised(126, 72, 150, 38, light=(60, 64, 72), shadow=(12, 13, 15))

    # Bottom buttons from sheet y = 86:
    for bx, label in [(11, "add"), (43, "rem"), (72, "sel"), (101, "misc"), (229, "opts")]:
        c.fill_rect(bx, 86, 24, 18, (40, 42, 48))
        c.bevel_raised(bx, 86, 24, 18, light=(75, 80, 90), shadow=(12, 13, 16))
        draw_string(c, label, bx + 2, 92, fg=(240, 242, 248), bg=(40, 42, 48))

    # Visualizer background at (205, 0, 75, 38)
    c.fill_rect(205, 0, 75, 38, (11, 12, 14))
    c.bevel_sunken(205, 0, 75, 38, shadow=(8, 9, 10), light=(45, 48, 55))

    return c.to_bmp()

def generate_rust_eqmain():
    """Generates eqmain.bmp (275 x 315). S01 §5, line 692. Includes Rust gear motif."""
    c = Canvas(275, 315, bg_color=(20, 21, 24))

    # Window background at (0, 0, 275, 116)
    c.texture_metal(0, 0, 275, 116, base_color=(23, 24, 28))
    c.bevel_raised(0, 0, 275, 116, light=(62, 66, 73), shadow=(13, 14, 16))

    draw_string(c, "+12db", 42, 38, fg=(110, 115, 125), bg=(23, 24, 28))
    draw_string(c, " 0db", 42, 63, fg=(110, 115, 125), bg=(23, 24, 28))
    draw_string(c, "-12db", 42, 88, fg=(110, 115, 125), bg=(23, 24, 28))

    band_labels = ["60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"]
    for i, lbl in enumerate(band_labels):
        bx = 79 + i * 18 - 4
        draw_string(c, lbl, bx, 102, fg=(110, 115, 125), bg=(23, 24, 28))

    draw_string(c, "pre", 20, 102, fg=(110, 115, 125), bg=(23, 24, 28))

    for k in [-1] + list(range(10)):
        gx = 22 if k == -1 else 79 + k * 18
        c.fill_rect(gx + 4, 38, 3, 55, (12, 13, 15))
        c.bevel_sunken(gx + 4, 38, 3, 55, shadow=(8, 8, 9), light=(45, 48, 55))

    # Title bars
    c.fill_rect(0, 134, 275, 14, (48, 22, 14))
    c.bevel_raised(0, 134, 275, 14, light=(90, 45, 25), shadow=(15, 10, 8))
    draw_string(c, "equalizer", 80, 138, fg=(255, 200, 160), bg=(48, 22, 14))

    c.fill_rect(0, 149, 275, 14, (32, 34, 38))
    c.bevel_raised(0, 149, 275, 14, light=(55, 58, 65), shadow=(12, 13, 15))
    draw_string(c, "equalizer", 80, 153, fg=(120, 125, 135), bg=(32, 34, 38))

    # Slider track area at (13, 164, 209, 129)
    c.fill_rect(13, 164, 209, 129, (18, 19, 22))

    # Slider thumbs
    c.fill_rect(0, 164, 11, 11, (55, 58, 66))
    c.bevel_raised(0, 164, 11, 11, light=(95, 100, 115), shadow=(18, 20, 24))
    c.hline(2, 169, 7, (25, 28, 32))
    c.hline(2, 170, 7, (90, 95, 105))

    c.fill_rect(0, 176, 11, 11, (42, 45, 52))
    c.bevel_sunken(0, 176, 11, 11, shadow=(14, 15, 18), light=(85, 90, 100))
    c.hline(2, 181, 7, (255, 107, 43))

    # ON, AUTO buttons with Rust orange lit indicators
    for bx, is_eng, is_dep in [(10, False, False), (69, True, False), (128, False, True), (187, True, True)]:
        bg = (28, 30, 35) if is_dep else (40, 42, 48)
        c.fill_rect(bx, 119, 26, 12, bg)
        if is_dep:
            c.bevel_sunken(bx, 119, 26, 12, shadow=(10, 11, 13), light=(60, 65, 75))
        else:
            c.bevel_raised(bx, 119, 26, 12, light=(75, 80, 90), shadow=(12, 13, 16))
        c.fill_rect(bx + 2, 123, 3, 3, (255, 107, 43) if is_eng else (45, 20, 12))
        draw_string(c, "on", bx + 8, 122, fg=(240, 242, 248), bg=bg)

    for bx, is_eng, is_dep in [(36, False, False), (95, True, False), (154, False, True), (213, True, True)]:
        bg = (28, 30, 35) if is_dep else (40, 42, 48)
        c.fill_rect(bx, 119, 32, 12, bg)
        if is_dep:
            c.bevel_sunken(bx, 119, 32, 12, shadow=(10, 11, 13), light=(60, 65, 75))
        else:
            c.bevel_raised(bx, 119, 32, 12, light=(75, 80, 90), shadow=(12, 13, 16))
        c.fill_rect(bx + 2, 123, 3, 3, (255, 107, 43) if is_eng else (45, 20, 12))
        draw_string(c, "auto", bx + 7, 122, fg=(240, 242, 248), bg=bg)

    for by, is_sel in [(164, False), (176, True)]:
        bg = (32, 34, 40) if is_sel else (40, 42, 48)
        c.fill_rect(224, by, 44, 12, bg)
        if is_sel:
            c.bevel_sunken(224, by, 44, 12, shadow=(10, 11, 13), light=(60, 65, 75))
        else:
            c.bevel_raised(224, by, 44, 12, light=(75, 80, 90), shadow=(12, 13, 16))
        draw_string(c, "presets", 227, by + 3, fg=(240, 242, 248), bg=bg)

    # Graph background at (0, 294, 113, 19)
    c.fill_rect(0, 294, 113, 19, (11, 12, 14))
    c.bevel_sunken(0, 294, 113, 19, shadow=(6, 7, 8), light=(40, 44, 50))
    c.hline(2, 303, 109, (28, 20, 16))

    # --- RUST MOTIF: Procedural Gear / Cog silhouette watermark in EQ graph ---
    c.draw_gear(cx=56, cy=303, r_outer=8, r_inner=5, r_hole=2, teeth=8, color=(140, 59, 24), hole_color=(11, 12, 14))

    # Graph line scale at (115, 294, 1, 19)
    for y in range(19):
        t = y / 18.0
        r = int(255 * (1.0 - t * 0.3))
        g = int(200 * t)
        b = int(30 * (1.0 - t))
        c.set(115, 294 + y, (r, g, b))

    # Preamp line at (0, 314, 113, 1) in Rust orange
    c.hline(0, 314, 113, (255, 107, 43))

    return c.to_bmp()

def generate_rust_eq_ex():
    """Generates eq_ex.bmp (275 x 82). S01 §5, line 693; fixture exact."""
    c = Canvas(275, 82, bg_color=(20, 21, 24))

    # Shade backgrounds: Focused (0, 0, 275, 14), Unfocused (0, 15, 275, 14)
    c.fill_rect(0, 0, 275, 14, (48, 22, 14))
    c.bevel_raised(0, 0, 275, 14, light=(90, 45, 25), shadow=(15, 10, 8))
    draw_string(c, "equalizer", 80, 4, fg=(255, 200, 160), bg=(48, 22, 14))

    c.fill_rect(0, 15, 275, 14, (32, 34, 38))
    c.bevel_raised(0, 15, 275, 14, light=(55, 58, 65), shadow=(12, 13, 15))
    draw_string(c, "equalizer", 80, 19, fg=(120, 125, 135), bg=(32, 34, 38))

    # Mini volume / balance sliders & buttons
    c.fill_rect(1, 30, 9, 7, (55, 58, 66))
    c.fill_rect(11, 30, 9, 7, (55, 58, 66))

    for by in [38, 47, 56, 65]:
        c.fill_rect(1, by, 9, 8, (40, 42, 48))
        c.fill_rect(11, by, 9, 8, (40, 42, 48))

    return c.to_bmp()

def generate_rust_gen():
    """Generates gen.bmp (194 x 109). S01 §5, line 694; fixture exact. Includes gear teeth border motif."""
    c = Canvas(194, 109, bg_color=(20, 21, 24))

    # Top border tiles (selected y=0, normal y=21)
    for base_y, focused in [(0, True), (21, False)]:
        bg = (48, 22, 14) if focused else (32, 34, 38)
        gear_col = (110, 48, 22) if focused else (50, 52, 58)
        # Corners & fill tiles
        for tx in range(0, 182, 26):
            c.fill_rect(tx, base_y, 25, 20, bg)
            c.bevel_raised(tx, base_y, 25, 20, light=(90, 45, 25) if focused else (55, 58, 65), shadow=(15, 10, 8))
            # Decorative gear tooth pattern along top edge
            for tooth in range(4):
                c.fill_rect(tx + 3 + tooth * 5, base_y + 2, 3, 2, gear_col)

    # Bottom bar at (0, 42, 125, 14) and (0, 57, 125, 14)
    c.fill_rect(0, 42, 125, 14, (28, 30, 35))
    c.bevel_raised(0, 42, 125, 14, light=(60, 64, 72), shadow=(12, 13, 15))
    c.fill_rect(0, 57, 125, 14, (28, 30, 35))
    c.bevel_raised(0, 57, 125, 14, light=(60, 64, 72), shadow=(12, 13, 15))

    # Middle rails at (127, 42, 11, 29) and (139, 42, 8, 29)
    c.fill_rect(127, 42, 11, 29, (28, 30, 35))
    c.fill_rect(139, 42, 8, 29, (18, 19, 22))

    return c.to_bmp()

def generate_rust_genex():
    """Generates genex.bmp (130 x 75). S01 §5, line 695; fixture exact."""
    c = Canvas(130, 75, bg_color=(20, 21, 24))
    c.texture_metal(0, 0, 130, 75, base_color=(23, 24, 28))
    c.bevel_raised(0, 0, 130, 75, light=(62, 66, 73), shadow=(13, 14, 16))

    # Extra generic controls & sliders
    c.fill_rect(10, 10, 110, 16, (14, 15, 17))
    c.bevel_sunken(10, 10, 110, 16, shadow=(8, 9, 10), light=(45, 48, 55))
    c.draw_gear(cx=65, cy=18, r_outer=6, r_inner=4, r_hole=2, teeth=6, color=(140, 59, 24), hole_color=(14, 15, 17))

    # Slider buttons & checkboxes with Rust orange active state
    c.fill_rect(10, 36, 12, 12, (40, 42, 48))
    c.bevel_raised(10, 36, 12, 12, light=(75, 80, 90), shadow=(12, 13, 16))
    c.fill_rect(13, 39, 6, 6, (255, 107, 43))

    c.fill_rect(30, 36, 12, 12, (28, 30, 35))
    c.bevel_sunken(30, 36, 12, 12, shadow=(10, 11, 13), light=(60, 65, 75))

    return c.to_bmp()

def generate_rust_mb():
    """Generates mb.bmp (234 x 119). S01 §5, line 696; fixture exact."""
    c = Canvas(234, 119, bg_color=(20, 21, 24))
    c.texture_metal(0, 0, 234, 119, base_color=(23, 24, 28))
    c.bevel_raised(0, 0, 234, 119, light=(62, 66, 73), shadow=(13, 14, 16))

    # Minibrowser titlebar (0, 0, 234, 14)
    c.fill_rect(0, 0, 234, 14, (48, 22, 14))
    c.bevel_raised(0, 0, 234, 14, light=(90, 45, 25), shadow=(15, 10, 8))
    draw_string(c, "browser", 10, 4, fg=(255, 200, 160), bg=(48, 22, 14))

    # Navigation buttons: Back, Forward, Stop, Reload, Home at y=20
    nav_buttons = [(" < ", 10), (" > ", 35), (" X ", 60), (" R ", 85), (" H ", 110)]
    for lbl, bx in nav_buttons:
        c.fill_rect(bx, 20, 22, 14, (40, 42, 48))
        c.bevel_raised(bx, 20, 22, 14, light=(75, 80, 90), shadow=(12, 13, 16))
        draw_string(c, lbl.strip(), bx + 7, 24, fg=(255, 107, 43), bg=(40, 42, 48))

    # Address bar frame at (10, 40, 214, 16)
    c.fill_rect(10, 40, 214, 16, (11, 12, 14))
    c.bevel_sunken(10, 40, 214, 16, shadow=(8, 9, 10), light=(45, 48, 55))
    draw_string(c, "http://rustamp", 15, 45, fg=(140, 70, 35), bg=(11, 12, 14))

    # Content window area at (10, 62, 214, 48)
    c.fill_rect(10, 62, 214, 48, (14, 15, 17))
    c.bevel_sunken(10, 62, 214, 48, shadow=(8, 9, 10), light=(45, 48, 55))
    c.draw_gear(cx=117, cy=86, r_outer=10, r_inner=6, r_hole=2, teeth=8, color=(70, 32, 16), hole_color=(14, 15, 17))

    return c.to_bmp()

def generate_rust_video():
    """Generates video.bmp (234 x 119). S01 §5, line 697; fixture exact."""
    c = Canvas(234, 119, bg_color=(20, 21, 24))
    c.texture_metal(0, 0, 234, 119, base_color=(23, 24, 28))
    c.bevel_raised(0, 0, 234, 119, light=(62, 66, 73), shadow=(13, 14, 16))

    # Video titlebar
    c.fill_rect(0, 0, 234, 14, (48, 22, 14))
    c.bevel_raised(0, 0, 234, 14, light=(90, 45, 25), shadow=(15, 10, 8))
    draw_string(c, "video", 10, 4, fg=(255, 200, 160), bg=(48, 22, 14))

    # Video screen viewport cutout at (10, 20, 214, 68)
    c.fill_rect(10, 20, 214, 68, (8, 9, 10))
    c.bevel_sunken(10, 20, 214, 68, shadow=(6, 7, 8), light=(40, 44, 50))
    # Gear watermark on screen
    c.draw_gear(cx=117, cy=54, r_outer=14, r_inner=9, r_hole=3, teeth=8, color=(45, 20, 12), hole_color=(8, 9, 10))

    # Video controls at bottom (10, 94, 214, 18)
    c.fill_rect(10, 94, 214, 18, (28, 30, 35))
    c.bevel_sunken(10, 94, 214, 18, shadow=(10, 11, 13), light=(50, 54, 60))

    # Play, Pause, Stop, Fullscreen buttons
    c.fill_rect(14, 96, 18, 14, (40, 42, 48))
    c.bevel_raised(14, 96, 18, 14, light=(75, 80, 90), shadow=(12, 13, 16))
    # Play icon in Rust orange
    for col in range(6):
        c.vline(19 + col, 99 + col // 2, 7 - col, (255, 107, 43))

    # Fullscreen button at x=200
    c.fill_rect(200, 96, 20, 14, (40, 42, 48))
    c.bevel_raised(200, 96, 20, 14, light=(75, 80, 90), shadow=(12, 13, 16))
    c.frame_rect(204, 100, 12, 6, (255, 107, 43))

    return c.to_bmp()


# --- Rust Theme Cursor Canvas & Drawing Helpers ---

class CursorCanvas:
    def __init__(self, width=32, height=32):
        self.width = width
        self.height = height
        self.pixels = [[(0, 0, 0, 0) for _ in range(width)] for _ in range(height)]

    def set(self, x, y, rgba):
        if 0 <= x < self.width and 0 <= y < self.height:
            self.pixels[y][x] = rgba

    def fill_rect(self, x, y, w, h, rgba):
        for cy in range(max(0, y), min(self.height, y + h)):
            for cx in range(max(0, x), min(self.width, x + w)):
                self.pixels[cy][cx] = rgba

    def hline(self, x, y, w, rgba):
        self.fill_rect(x, y, w, 1, rgba)

    def vline(self, x, y, h, rgba):
        self.fill_rect(x, y, 1, h, rgba)

    def frame_rect(self, x, y, w, h, rgba):
        self.hline(x, y, w, rgba)
        self.hline(x, y + h - 1, w, rgba)
        self.vline(x, y, h, rgba)
        self.vline(x + w - 1, y, h, rgba)

    def to_cur(self, hotspot_x, hotspot_y):
        return create_cur_32(self.width, self.height, hotspot_x, hotspot_y, lambda x, y: self.pixels[y][x])


# Standard Rust palette for cursors
CUR_BLACK = (0, 0, 0, 255)
CUR_WHITE = (240, 242, 248, 255)
CUR_ORANGE = (255, 107, 43, 255)
CUR_ORANGE_DARK = (180, 60, 20, 255)

def draw_arrow_pointer(c, hx=0, hy=0, accent_color=CUR_ORANGE):
    """Draws a standard pointer arrow with black outline, white body, and Rust accent."""
    for y in range(16):
        for x in range(y + 1):
            c.set(hx + x, hy + y, CUR_WHITE)
    # Orange spine accent
    for d in range(2, 12):
        c.set(hx + 1, hy + d, accent_color)
    # Outline
    for y in range(17):
        c.set(hx, hy + y, CUR_BLACK)
        c.set(hx + y, hy + y, CUR_BLACK)
    c.set(hx + 1, hy + 16, CUR_BLACK)

def generate_rust_normal_cur():
    """Generates normal.cur (32 x 32, hotspot 0, 0)."""
    c = CursorCanvas()
    draw_arrow_pointer(c, 0, 0, CUR_ORANGE)
    return c.to_cur(0, 0)

def generate_rust_pnormal_cur():
    """Generates pnormal.cur (32 x 32, hotspot 0, 0)."""
    c = CursorCanvas()
    draw_arrow_pointer(c, 0, 0, CUR_ORANGE)
    # Playlist icon accent (mini list lines) at (14, 12)
    c.fill_rect(13, 11, 8, 8, CUR_BLACK)
    c.fill_rect(14, 12, 6, 6, (32, 34, 40, 255))
    c.hline(15, 13, 4, CUR_ORANGE)
    c.hline(15, 15, 4, CUR_ORANGE)
    c.hline(15, 17, 3, CUR_ORANGE)
    return c.to_cur(0, 0)

def generate_rust_close_cur():
    """Generates close.cur (32 x 32, hotspot 4, 4)."""
    c = CursorCanvas()
    # Close 'X' symbol in Rust orange with black border
    c.fill_rect(0, 0, 9, 9, CUR_BLACK)
    c.fill_rect(1, 1, 7, 7, (45, 48, 55, 255))
    for d in range(5):
        c.set(2 + d, 2 + d, CUR_ORANGE)
        c.set(6 - d, 2 + d, CUR_ORANGE)
    return c.to_cur(4, 4)

def generate_rust_titlebar_cur():
    """Generates titlebar.cur (32 x 32, hotspot 8, 8). 4-way move cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    # 4-way arrows
    for d in range(-6, 7):
        c.set(cx + d, cy, CUR_WHITE)
        c.set(cx, cy + d, CUR_WHITE)
    # Arrow heads in Rust orange
    c.set(cx, cy - 7, CUR_ORANGE)
    c.hline(cx - 1, cy - 6, 3, CUR_ORANGE)
    c.set(cx, cy + 7, CUR_ORANGE)
    c.hline(cx - 1, cy + 6, 3, CUR_ORANGE)
    c.set(cx - 7, cy, CUR_ORANGE)
    c.vline(cx - 6, cy - 1, 3, CUR_ORANGE)
    c.set(cx + 7, cy, CUR_ORANGE)
    c.vline(cx + 6, cy - 1, 3, CUR_ORANGE)
    # Black borders around heads
    c.set(cx, cy, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_posbar_cur():
    """Generates posbar.cur (32 x 32, hotspot 8, 8). Horizontal slider cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    c.hline(cx - 7, cy, 15, CUR_WHITE)
    c.vline(cx, cy - 3, 7, CUR_WHITE)
    # Horizontal arrow heads in Rust orange
    c.set(cx - 7, cy, CUR_ORANGE)
    c.vline(cx - 6, cy - 2, 5, CUR_ORANGE)
    c.vline(cx - 5, cy - 1, 3, CUR_ORANGE)
    c.set(cx + 7, cy, CUR_ORANGE)
    c.vline(cx + 6, cy - 2, 5, CUR_ORANGE)
    c.vline(cx + 5, cy - 1, 3, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_volbal_cur():
    """Generates volbal.cur (32 x 32, hotspot 8, 8). Horizontal volume/balance cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    c.hline(cx - 6, cy, 13, CUR_WHITE)
    # Volume wedge arc ticks in Rust orange
    c.vline(cx - 2, cy - 3, 7, CUR_ORANGE)
    c.vline(cx + 2, cy - 4, 9, CUR_ORANGE)
    # Left and right arrow tips
    c.set(cx - 7, cy, CUR_ORANGE)
    c.set(cx + 7, cy, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_eqslid_cur():
    """Generates eqslid.cur (32 x 32, hotspot 8, 8). Vertical slider cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    c.vline(cx, cy - 7, 15, CUR_WHITE)
    c.hline(cx - 3, cy, 7, CUR_WHITE)
    # Vertical arrow heads in Rust orange
    c.set(cx, cy - 7, CUR_ORANGE)
    c.hline(cx - 2, cy - 6, 5, CUR_ORANGE)
    c.hline(cx - 1, cy - 5, 3, CUR_ORANGE)
    c.set(cx, cy + 7, CUR_ORANGE)
    c.hline(cx - 2, cy + 6, 5, CUR_ORANGE)
    c.hline(cx - 1, cy + 5, 3, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_eqtitle_cur():
    """Generates eqtitle.cur (32 x 32, hotspot 8, 8). Equalizer title drag cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    # 4-way move with mini EQ graph
    c.hline(cx - 6, cy, 13, CUR_WHITE)
    c.vline(cx, cy - 6, 13, CUR_WHITE)
    c.set(cx, cy - 7, CUR_ORANGE)
    c.set(cx, cy + 7, CUR_ORANGE)
    c.set(cx - 7, cy, CUR_ORANGE)
    c.set(cx + 7, cy, CUR_ORANGE)
    # Mini EQ bars in corners
    c.vline(cx + 4, cy - 4, 3, CUR_ORANGE)
    c.vline(cx + 6, cy - 5, 4, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_psize_cur():
    """Generates psize.cur (32 x 32, hotspot 8, 8). Diagonal resize cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    # Diagonal line from (-6, -6) to (6, 6)
    for d in range(-6, 7):
        c.set(cx + d, cy + d, CUR_WHITE)
        c.set(cx + d + 1, cy + d, CUR_BLACK)
    # Arrowheads in Rust orange
    c.set(cx - 7, cy - 7, CUR_ORANGE)
    c.set(cx - 6, cy - 7, CUR_ORANGE)
    c.set(cx - 7, cy - 6, CUR_ORANGE)
    c.set(cx + 7, cy + 7, CUR_ORANGE)
    c.set(cx + 6, cy + 7, CUR_ORANGE)
    c.set(cx + 7, cy + 6, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_ptbar_cur():
    """Generates ptbar.cur (32 x 32, hotspot 8, 8). Playlist titlebar move cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    c.hline(cx - 6, cy, 13, CUR_WHITE)
    c.vline(cx, cy - 6, 13, CUR_WHITE)
    c.set(cx, cy - 7, CUR_ORANGE)
    c.set(cx, cy + 7, CUR_ORANGE)
    c.set(cx - 7, cy, CUR_ORANGE)
    c.set(cx + 7, cy, CUR_ORANGE)
    # Mini playlist lines
    c.hline(cx + 3, cy - 3, 4, CUR_ORANGE)
    c.hline(cx + 3, cy - 1, 4, CUR_ORANGE)
    return c.to_cur(cx, cy)

def generate_rust_pvscroll_cur():
    """Generates pvscroll.cur (32 x 32, hotspot 8, 8). Vertical scroll cursor."""
    c = CursorCanvas()
    cx, cy = 8, 8
    c.vline(cx, cy - 7, 15, CUR_WHITE)
    c.set(cx, cy - 7, CUR_ORANGE)
    c.hline(cx - 2, cy - 6, 5, CUR_ORANGE)
    c.set(cx, cy + 7, CUR_ORANGE)
    c.hline(cx - 2, cy + 6, 5, CUR_ORANGE)
    # Scrollbar thumb box in center
    c.frame_rect(cx - 3, cy - 2, 7, 5, CUR_WHITE)
    c.fill_rect(cx - 2, cy - 1, 5, 3, CUR_ORANGE)
    return c.to_cur(cx, cy)


# --- Text File Generators ---

def generate_rust_pledit_txt():
    """Generates pledit.txt in Rust theme palette."""
    return (
        "[Text]\r\n"
        "Normal=#DF7B38\r\n"
        "Current=#FFFFFF\r\n"
        "NormalBG=#141517\r\n"
        "SelectedBG=#0000C6\r\n"
        "Font=Arial\r\n"
    ).encode('utf-8')

def generate_rust_viscolor_txt():
    """
    Generates viscolor.txt (24 fire/rust gradient colors).
    Slot 0: background (obsidian black)
    Slot 1: grid lines (dark charcoal rust)
    Slots 2-17: 16-step fire/rust spectrum gradient (white-amber -> rust flame -> deep ember)
    Slots 18-22: 5 oscilloscope trace colors (bright flame amber -> deep rust)
    Slot 23: peak-dots (bright blazing flame)
    """
    lines = [
        "12,13,14",     # 0: Analyser background (obsidian black)
        "38,32,29",     # 1: Grid lines (dark charcoal rust)
        "255,240,208",  # 2: Top spectrum peak (white-hot amber)
        "255,210,128",  # 3: Bright flame gold
        "255,180,64",   # 4: Warm amber
        "255,145,36",   # 5: Blazing flame
        "255,117,24",   # 6: Vibrant fire
        "240,94,22",    # 7: Vivid Rust orange
        "229,90,28",    # 8: Rust logo orange
        "217,68,18",    # 9: Warm rust
        "204,56,16",    # 10: Deep flame
        "184,44,14",    # 11: Burnished rust
        "163,34,12",    # 12: Ember orange
        "142,26,10",    # 13: Dark rust
        "122,20,8",     # 14: Deep ember
        "102,16,6",     # 15: Low ember
        "82,12,4",      # 16: Dim furnace glow
        "62,8,2",       # 17: Bottom baseline ember
        "255,179,71",   # 18: Scope 1 (brightest flame amber)
        "255,140,26",   # 19: Scope 2
        "230,106,13",   # 20: Scope 3
        "179,74,0",     # 21: Scope 4
        "128,48,0",     # 22: Scope 5
        "255,224,130",  # 23: Peak dots (blazing white-gold)
    ]
    return ("\r\n".join(lines) + "\r\n").encode('utf-8')

def generate_rust_region_txt():
    """Generates region.txt."""
    return (
        "[Normal]\r\n"
        "NumPoints=4\r\n"
        "PointList=0,0, 275,0, 275,116, 0,116\r\n"
        "[WindowShade]\r\n"
        "NumPoints=4\r\n"
        "PointList=0,0, 275,0, 275,14, 0,14\r\n"
        "[Equalizer]\r\n"
        "NumPoints=4\r\n"
        "PointList=0,0, 275,0, 275,116, 0,116\r\n"
    ).encode('utf-8')


# --- Master Specifications Table ---

RUST_THEME_GENERATORS = {
    # 18 classic sheets + 1 extended sheet
    "main.bmp": generate_rust_main,
    "titlebar.bmp": generate_rust_titlebar,
    "cbuttons.bmp": generate_rust_cbuttons,
    "shufrep.bmp": generate_rust_shufrep,
    "posbar.bmp": generate_rust_posbar,
    "volume.bmp": generate_rust_volume,
    "balance.bmp": generate_rust_balance,
    "monoster.bmp": generate_rust_monoster,
    "playpaus.bmp": generate_rust_playpaus,
    "numbers.bmp": generate_rust_numbers,
    "nums_ex.bmp": generate_rust_nums_ex,
    "text.bmp": generate_rust_text,
    "pledit.bmp": generate_rust_pledit,
    "eqmain.bmp": generate_rust_eqmain,
    "eq_ex.bmp": generate_rust_eq_ex,
    "gen.bmp": generate_rust_gen,
    "genex.bmp": generate_rust_genex,
    "mb.bmp": generate_rust_mb,
    "video.bmp": generate_rust_video,
    # 11 cursors
    "normal.cur": generate_rust_normal_cur,
    "pnormal.cur": generate_rust_pnormal_cur,
    "close.cur": generate_rust_close_cur,
    "titlebar.cur": generate_rust_titlebar_cur,
    "posbar.cur": generate_rust_posbar_cur,
    "volbal.cur": generate_rust_volbal_cur,
    "eqslid.cur": generate_rust_eqslid_cur,
    "eqtitle.cur": generate_rust_eqtitle_cur,
    "psize.cur": generate_rust_psize_cur,
    "ptbar.cur": generate_rust_ptbar_cur,
    "pvscroll.cur": generate_rust_pvscroll_cur,
    # 3 text files
    "pledit.txt": generate_rust_pledit_txt,
    "region.txt": generate_rust_region_txt,
    "viscolor.txt": generate_rust_viscolor_txt,
}

# Expected pixel-exact dimensions per specs/S01-window-geometry.md and base-2.91.wsz fixture
EXPECTED_DIMENSIONS = {
    "main.bmp": (275, 116),
    "titlebar.bmp": (344, 87),
    "cbuttons.bmp": (136, 36),
    "shufrep.bmp": (92, 85),
    "posbar.bmp": (307, 10),
    "volume.bmp": (68, 433),
    "balance.bmp": (68, 433),
    "monoster.bmp": (58, 24),
    "playpaus.bmp": (42, 9),
    "numbers.bmp": (99, 13),
    "nums_ex.bmp": (108, 13),
    "text.bmp": (155, 74),
    "pledit.bmp": (280, 186),
    "eqmain.bmp": (275, 315),
    "eq_ex.bmp": (275, 82),
    "gen.bmp": (194, 109),
    "genex.bmp": (130, 75),
    "mb.bmp": (234, 119),
    "video.bmp": (234, 119),
    "normal.cur": (32, 32),
    "pnormal.cur": (32, 32),
    "close.cur": (32, 32),
    "titlebar.cur": (32, 32),
    "posbar.cur": (32, 32),
    "volbal.cur": (32, 32),
    "eqslid.cur": (32, 32),
    "eqtitle.cur": (32, 32),
    "psize.cur": (32, 32),
    "ptbar.cur": (32, 32),
    "pvscroll.cur": (32, 32),
}

def verify_static_dimensions(output_dir):
    """Static dimension check: asserts every generated file matches the required spec dimensions."""
    print("Verifying static dimensions...")
    for filename, expected_size in EXPECTED_DIMENSIONS.items():
        file_path = os.path.join(output_dir, filename)
        assert os.path.exists(file_path), f"Missing generated file: {filename}"
        with open(file_path, "rb") as f:
            data = f.read()

        if filename.endswith(".bmp"):
            assert len(data) >= 26, f"{filename} too small to be BMP"
            w, h = struct.unpack('<ii', data[18:26])
            actual = (w, abs(h))
            assert actual == expected_size, (
                f"Dimension mismatch in {filename}: expected {expected_size}, got {actual}"
            )
        elif filename.endswith(".cur"):
            assert len(data) >= 22, f"{filename} too small to be CUR"
            count = struct.unpack('<H', data[4:6])[0]
            assert count == 1, f"{filename} should contain 1 cursor image, got {count}"
            w, h = data[6], data[7]
            w = 256 if w == 0 else w
            h = 256 if h == 0 else h
            actual = (w, h)
            assert actual == expected_size, (
                f"Dimension mismatch in {filename}: expected {expected_size}, got {actual}"
            )

    # Verify text files non-empty
    for txt in ["pledit.txt", "region.txt", "viscolor.txt"]:
        file_path = os.path.join(output_dir, txt)
        assert os.path.exists(file_path), f"Missing text file: {txt}"
        assert os.path.getsize(file_path) > 0, f"Empty text file: {txt}"

    print(f"STATIC CHECK PASSED: All {len(EXPECTED_DIMENSIONS)} sheets and cursors match exact spec dimensions!")

def main():
    os.makedirs(RUST_THEME_DIR, exist_ok=True)
    print(f"Generating complete Rust-themed skin (32 entries + nums_ex.bmp) to {RUST_THEME_DIR}...")
    for filename, gen_fn in RUST_THEME_GENERATORS.items():
        out_path = os.path.join(RUST_THEME_DIR, filename)
        data = gen_fn()
        with open(out_path, "wb") as f:
            f.write(data)
        print(f"  {filename:16} -> {len(data):6} bytes")

    verify_static_dimensions(RUST_THEME_DIR)
    print("Done! Rust-themed skin generated and verified successfully.")

if __name__ == "__main__":
    main()
