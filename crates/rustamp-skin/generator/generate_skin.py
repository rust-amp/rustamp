#!/usr/bin/env python3
"""
Procedural generator for the Rustamp lookalike default skin.

Clean-room design:
- Draws classic dark metallic player chrome, buttons, displays, sliders, and fonts from scratch.
- Purely procedural: no external images, fonts, or assets.
- Produces all 16 BMP sheets expected by the Rustamp skin loader at exact spec dimensions.
"""

import os
import struct
import math

# Output directory for generated sheets
OUTPUT_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "lookalike")

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

class Canvas:
    def __init__(self, width, height, bg_color=(36, 38, 44)):
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

    def bevel_raised(self, x, y, w, h, light=(90, 95, 105), shadow=(18, 20, 24)):
        self.hline(x, y, w, light)
        self.vline(x, y, h, light)
        self.hline(x, y + h - 1, w, shadow)
        self.vline(x + w - 1, y, h, shadow)

    def bevel_sunken(self, x, y, w, h, shadow=(18, 20, 24), light=(75, 80, 90)):
        self.hline(x, y, w, shadow)
        self.vline(x, y, h, shadow)
        self.hline(x, y + h - 1, w, light)
        self.vline(x + w - 1, y, h, light)

    def texture_metal(self, x, y, w, h, base_color=(38, 40, 46)):
        for cy in range(y, y + h):
            if cy < 0 or cy >= self.height:
                continue
            delta = (cy % 2) * 4 - 2
            r = max(0, min(255, base_color[0] + delta))
            g = max(0, min(255, base_color[1] + delta))
            b = max(0, min(255, base_color[2] + delta))
            for cx in range(x, x + w):
                if 0 <= cx < self.width:
                    self.pixels[cy][cx] = (r, g, b)

    def to_bmp(self):
        return create_bmp_24(self.width, self.height, lambda x, y: self.pixels[y][x])


# 5x6 bitmap font definition for text.bmp
# 31 columns x 3 rows:
# Row 0: a..z, ", @, unused, unused, space
# Row 1: 0..9, …, ., :, (, ), -, ', !, _, +, \, /, [, ], ^, &, %, ,, =, $, #
# Row 2: Å, Ö, Ä, ?, *
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

def draw_text_glyph(canvas, char, x, y, fg=(0, 255, 60), bg=(0, 0, 0)):
    grid = FONT_5X6.get(char.lower())
    if not grid:
        grid = FONT_5X6.get('?')
    for cy in range(6):
        line = grid[cy]
        for cx in range(5):
            canvas.set(x + cx, y + cy, fg if line[cx] == '1' else bg)

def draw_string(canvas, text, x, y, fg=(0, 255, 60), bg=(0, 0, 0)):
    cur_x = x
    for ch in text:
        draw_text_glyph(canvas, ch, cur_x, y, fg, bg)
        cur_x += 5

# 7-Segment digit patterns for 9x13 cells
# Segments:
#   aaa
#  f   b
#  f   b
#   ggg
#  e   c
#  e   c
#   ddd
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

def draw_7segment_cell(canvas, char, x, y, lit_color=(0, 255, 60), dim_color=(12, 35, 18), bg_color=(5, 8, 8)):
    canvas.fill_rect(x, y, 9, 13, bg_color)
    segs = SEGMENTS_7.get(char, set())
    
    # Segment positions (9x13)
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


# --- Individual Sheet Generators ---

def generate_main():
    """Generates main.bmp (275 x 116)."""
    c = Canvas(275, 116)
    # Dark metallic chassis
    c.texture_metal(0, 0, 275, 116, base_color=(40, 42, 48))
    c.bevel_raised(0, 0, 275, 116, light=(85, 90, 100), shadow=(15, 16, 20))

    # Main display cutout: (10, 22) to (264, 68)
    c.fill_rect(10, 22, 255, 46, (8, 10, 12))
    c.bevel_sunken(10, 22, 255, 46, shadow=(14, 16, 18), light=(70, 75, 85))

    # Static colon in display for time between 60+9=69 and 78
    c.fill_rect(73, 29, 2, 2, (0, 255, 60))
    c.fill_rect(73, 35, 2, 2, (0, 255, 60))

    # Static label indicators in display:
    # kbps label at window (130, 43), kHz label at (175, 43)
    draw_string(c, "kbps", 130, 43, fg=(45, 90, 55), bg=(8, 10, 12))
    draw_string(c, "khz", 175, 43, fg=(45, 90, 55), bg=(8, 10, 12))

    # Position bar groove at (16, 72, 248, 10)
    c.fill_rect(16, 72, 248, 10, (14, 15, 18))
    c.bevel_sunken(16, 72, 248, 10, shadow=(8, 8, 10), light=(65, 70, 80))

    # Volume slider groove at (107, 57, 68, 14)
    c.fill_rect(107, 57, 68, 14, (12, 14, 16))
    c.bevel_sunken(107, 57, 68, 14, shadow=(8, 8, 10), light=(65, 70, 80))

    # Balance slider groove at (177, 57, 38, 14)
    c.fill_rect(177, 57, 38, 14, (12, 14, 16))
    c.bevel_sunken(177, 57, 38, 14, shadow=(8, 8, 10), light=(65, 70, 80))

    # Transport button wells at (16, 88) to (136, 106)
    c.bevel_sunken(15, 87, 143, 20, shadow=(15, 16, 20), light=(70, 75, 85))

    return c.to_bmp()

def generate_titlebar():
    """Generates titlebar.bmp (350 x 87)."""
    c = Canvas(350, 87, bg_color=(20, 22, 26))

    # 1. Focused titlebar at (27, 0, 275, 14)
    for x in range(27, 302):
        t = (x - 27) / 275.0
        # Deep sapphire metallic blue gradient
        r = int(20 + 15 * (1.0 - t))
        g = int(35 + 20 * (1.0 - t))
        b = int(90 + 40 * (1.0 - t))
        for y in range(0, 14):
            line_hl = 15 if (y == 1 or y == 12) else (0 if y != 6 else -10)
            c.set(x, y, (min(255, r + line_hl), min(255, g + line_hl), min(255, b + line_hl)))
    c.bevel_raised(27, 0, 275, 14, light=(100, 120, 190), shadow=(10, 15, 40))
    # Title text "RUSTAMP"
    draw_string(c, "rustamp", 80, 4, fg=(220, 235, 255), bg=(25, 45, 110))

    # 2. Unfocused titlebar at (27, 15, 275, 14)
    for x in range(27, 302):
        t = (x - 27) / 275.0
        r = int(45 - 12 * t)
        g = int(48 - 12 * t)
        b = int(55 - 14 * t)
        for y in range(15, 29):
            line_hl = 10 if (y == 16 or y == 27) else 0
            c.set(x, y, (r + line_hl, g + line_hl, b + line_hl))
    c.bevel_raised(27, 15, 275, 14, light=(80, 85, 95), shadow=(18, 20, 24))
    draw_string(c, "rustamp", 80, 19, fg=(150, 155, 165), bg=(42, 45, 52))

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

    # Embedded title bar window buttons in the title bars (close at 27+264=291, etc.)
    for base_y in [0, 15, 29, 42, 57, 72]:
        # Options button at x=27+3
        c.fill_rect(27 + 3, base_y + 3, 9, 8, (60, 65, 75))
        c.bevel_raised(27 + 3, base_y + 3, 9, 8, light=(120, 125, 140), shadow=(15, 18, 22))
        c.hline(27 + 5, base_y + 5, 5, (200, 210, 225))
        c.hline(27 + 5, base_y + 7, 5, (200, 210, 225))
        c.hline(27 + 5, base_y + 9, 5, (200, 210, 225))

        # Minimize at 27+244
        c.fill_rect(27 + 244, base_y + 3, 9, 8, (60, 65, 75))
        c.bevel_raised(27 + 244, base_y + 3, 9, 8, light=(120, 125, 140), shadow=(15, 18, 22))
        c.hline(27 + 246, base_y + 8, 5, (220, 230, 245))

        # Shade at 27+254
        c.fill_rect(27 + 254, base_y + 3, 9, 8, (60, 65, 75))
        c.bevel_raised(27 + 254, base_y + 3, 9, 8, light=(120, 125, 140), shadow=(15, 18, 22))
        c.frame_rect(27 + 256, base_y + 5, 5, 4, (220, 230, 245))

        # Close at 27+264
        c.fill_rect(27 + 264, base_y + 3, 9, 8, (60, 65, 75))
        c.bevel_raised(27 + 264, base_y + 3, 9, 8, light=(120, 125, 140), shadow=(15, 18, 22))
        # Draw X
        for d in range(5):
            c.set(27 + 266 + d, base_y + 5 + d, (220, 230, 245))
            c.set(27 + 270 - d, base_y + 5 + d, (220, 230, 245))

    # Separate window buttons at (0, 0) .. (26, 36)
    for bx, by, sym in [(0, 0, 'opt'), (9, 0, 'min'), (0, 18, 'shade'), (18, 0, 'close')]:
        c.fill_rect(bx, by, 9, 9, (60, 65, 75))
        c.bevel_raised(bx, by, 9, 9, light=(110, 115, 130), shadow=(18, 20, 24))
        # Depressed at (bx, by + 9) or (bx + 9, by)
        dep_x, dep_y = (bx, by + 9) if by == 0 else (bx + 9, by)
        c.fill_rect(dep_x, dep_y, 9, 9, (45, 48, 55))
        c.bevel_sunken(dep_x, dep_y, 9, 9, shadow=(18, 20, 24), light=(90, 95, 105))

    # Clutter bar background at (304, 0, 8, 43)
    c.fill_rect(304, 0, 8, 43, (28, 30, 35))
    c.bevel_sunken(304, 0, 8, 43, shadow=(15, 16, 20), light=(65, 70, 80))
    c.fill_rect(312, 0, 8, 43, (24, 25, 28)) # Disabled
    c.bevel_sunken(312, 0, 8, 43, shadow=(12, 14, 16), light=(50, 55, 60))

    # Clutter bar lit buttons:
    # O (308, 47, 4, 6), A (316, 55, 4, 6), I (324, 63, 3, 6), D (332, 71, 4, 6), V (340, 78, 4, 6)
    lit = (0, 255, 100)
    # O
    c.fill_rect(308, 47, 4, 6, (0, 0, 0))
    c.frame_rect(308, 47, 4, 6, lit)
    # A
    c.fill_rect(316, 55, 4, 6, (0, 0, 0))
    c.hline(317, 55, 2, lit)
    c.vline(316, 56, 5, lit)
    c.vline(319, 56, 5, lit)
    c.hline(316, 58, 4, lit)
    # I
    c.fill_rect(324, 63, 3, 6, (0, 0, 0))
    c.vline(325, 63, 6, lit)
    # D
    c.fill_rect(332, 71, 4, 6, (0, 0, 0))
    c.vline(332, 71, 6, lit)
    c.hline(332, 71, 3, lit)
    c.hline(332, 76, 3, lit)
    c.vline(335, 72, 4, lit)
    # V
    c.fill_rect(340, 78, 4, 6, (0, 0, 0))
    c.vline(340, 78, 4, lit)
    c.vline(343, 78, 4, lit)
    c.set(341, 82, lit)
    c.set(342, 82, lit)

    # Shade position thumb pieces at (0, 36)
    c.fill_rect(0, 36, 17, 7, (18, 20, 24))
    c.fill_rect(17, 36, 9, 7, (75, 80, 95))
    c.bevel_raised(17, 36, 9, 7, light=(120, 130, 150), shadow=(25, 28, 35))

    return c.to_bmp()

def generate_cbuttons():
    """Generates cbuttons.bmp (136 x 36)."""
    c = Canvas(136, 36, bg_color=(20, 22, 26))

    # 6 transport buttons:
    # Prev (0,0), Play (23,0), Pause (46,0), Stop (69,0), Next (92,0) -> 22 x 18
    # Eject (114,0) -> 22 x 16
    buttons = [
        (0, 0, 22, 18, 'prev'),
        (23, 0, 22, 18, 'play'),
        (46, 0, 22, 18, 'pause'),
        (69, 0, 22, 18, 'stop'),
        (92, 0, 22, 18, 'next'),
        (114, 0, 22, 16, 'eject'),
    ]

    for x, y, w, h, kind in buttons:
        # 1. Normal state (y=0)
        c.fill_rect(x, y, w, h, (55, 60, 68))
        c.bevel_raised(x, y, w, h, light=(95, 100, 115), shadow=(18, 20, 24))
        # 2. Active / pressed state (y=18, eject at y=16)
        py = 18 if kind != 'eject' else 16
        c.fill_rect(x, py, w, h, (42, 46, 52))
        c.bevel_sunken(x, py, w, h, shadow=(15, 16, 20), light=(80, 85, 95))

        # Icons (normal at cy=y, pressed at cy=py with +1 offset)
        for is_pressed in [False, True]:
            ox = x + (1 if is_pressed else 0)
            oy = (py if is_pressed else y) + (1 if is_pressed else 0)
            icon_col = (200, 215, 230) if not is_pressed else (160, 175, 190)

            if kind == 'prev':
                # Vertical bar at left, double triangle
                c.vline(ox + 4, oy + 4, 10, icon_col)
                # First triangle
                for row in range(5):
                    c.vline(ox + 9 - row, oy + 4 + row, (5 - row) * 2, icon_col)
                # Second triangle
                for row in range(5):
                    c.vline(ox + 15 - row, oy + 4 + row, (5 - row) * 2, icon_col)
            elif kind == 'play':
                # Right facing triangle
                for col in range(9):
                    h_t = max(1, 10 - col)
                    c.vline(ox + 6 + col, oy + 4 + col // 2, 10 - col, icon_col)
            elif kind == 'pause':
                # Two vertical bars
                c.fill_rect(ox + 6, oy + 4, 3, 10, icon_col)
                c.fill_rect(ox + 12, oy + 4, 3, 10, icon_col)
            elif kind == 'stop':
                # Square
                c.fill_rect(ox + 6, oy + 4, 10, 10, icon_col)
            elif kind == 'next':
                # Double triangle right, vertical bar at right
                for row in range(5):
                    c.vline(ox + 5 + row, oy + 4 + row, (5 - row) * 2, icon_col)
                for row in range(5):
                    c.vline(ox + 11 + row, oy + 4 + row, (5 - row) * 2, icon_col)
                c.vline(ox + 16, oy + 4, 10, icon_col)
            elif kind == 'eject':
                # Triangle up, bar bottom
                for row in range(5):
                    c.hline(ox + 10 - row, oy + 4 + row, row * 2 + 1, icon_col)
                c.hline(ox + 6, oy + 11, 10, icon_col)

    return c.to_bmp()

def generate_shufrep():
    """Generates shufrep.bmp (92 x 85)."""
    c = Canvas(92, 85, bg_color=(20, 22, 26))

    # Repeat (0, 0, 28, 15) states at y = 0, 15, 30, 45
    # Shuffle (28, 0, 47, 15) states at y = 0, 15, 30, 45
    for state, sy in enumerate([0, 15, 30, 45]):
        is_depressed = (state % 2 == 1)
        is_selected = (state >= 2)

        # Repeat
        bg = (42, 46, 52) if is_depressed else (55, 60, 68)
        c.fill_rect(0, sy, 28, 15, bg)
        if is_depressed:
            c.bevel_sunken(0, sy, 28, 15, shadow=(15, 16, 20), light=(80, 85, 95))
        else:
            c.bevel_raised(0, sy, 28, 15, light=(95, 100, 115), shadow=(18, 20, 24))

        # Shuffle
        c.fill_rect(28, sy, 47, 15, bg)
        if is_depressed:
            c.bevel_sunken(28, sy, 47, 15, shadow=(15, 16, 20), light=(80, 85, 95))
        else:
            c.bevel_raised(28, sy, 47, 15, light=(95, 100, 115), shadow=(18, 20, 24))

        # Labels / LED indicators
        led_col = (0, 255, 60) if is_selected else (20, 50, 25)
        # Repeat LED dot at (3, sy + 6)
        c.fill_rect(3, sy + 6, 3, 3, led_col)
        draw_string(c, "rep", 8, sy + 4, fg=(220, 230, 245), bg=bg)

        # Shuffle LED dot at (31, sy + 6)
        c.fill_rect(31, sy + 6, 3, 3, led_col)
        draw_string(c, "shuffle", 36, sy + 4, fg=(220, 230, 245), bg=bg)

    # EQ button (23 x 12) at:
    # Normal: (0, 61), Selected: (0, 73), Depressed: (46, 61), Depressed+Selected: (46, 73)
    # Playlist button (23 x 12) at:
    # Normal: (23, 61), Selected: (23, 73), Depressed: (69, 61), Depressed+Selected: (69, 73)
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
        bg = (42, 46, 52) if is_dep else (55, 60, 68)
        c.fill_rect(bx, by, 23, 12, bg)
        if is_dep:
            c.bevel_sunken(bx, by, 23, 12, shadow=(15, 16, 20), light=(80, 85, 95))
        else:
            c.bevel_raised(bx, by, 23, 12, light=(95, 100, 115), shadow=(18, 20, 24))

        fg = (0, 255, 60) if is_sel else (180, 190, 205)
        text = "eq" if kind == 'eq' else "pl"
        draw_string(c, text, bx + 6, by + 3, fg=fg, bg=bg)

    return c.to_bmp()

def generate_posbar():
    """Generates posbar.bmp (307 x 10)."""
    c = Canvas(307, 10, bg_color=(15, 16, 20))

    # Track at (0, 0, 248, 10)
    c.fill_rect(0, 0, 248, 10, (18, 20, 24))
    c.bevel_sunken(0, 0, 248, 10, shadow=(8, 10, 12), light=(60, 65, 75))
    # Groove line
    c.hline(2, 4, 244, (10, 12, 14))
    c.hline(2, 5, 244, (35, 38, 45))

    # Resting thumb at (246, 0, 29, 10)
    c.fill_rect(246, 0, 29, 10, (75, 80, 90))
    c.bevel_raised(246, 0, 29, 10, light=(130, 140, 155), shadow=(25, 28, 35))
    # Vertical grip lines
    for gx in [254, 258, 262, 266]:
        c.vline(gx, 2, 6, (120, 130, 145))
        c.vline(gx + 1, 2, 6, (35, 38, 45))

    # Dragged thumb at (278, 0, 29, 10)
    c.fill_rect(278, 0, 29, 10, (65, 70, 80))
    c.bevel_sunken(278, 0, 29, 10, shadow=(20, 22, 28), light=(110, 120, 135))
    for gx in [286, 290, 294, 298]:
        c.vline(gx, 2, 6, (40, 42, 50))
        c.vline(gx + 1, 2, 6, (130, 140, 155))

    return c.to_bmp()

def generate_volume():
    """Generates volume.bmp (68 x 433). 28 frames of 15 rows each + thumb strip at y=420."""
    c = Canvas(68, 433, bg_color=(15, 16, 20))

    # 28 frames of volume track (68 x 15)
    for frame in range(28):
        fy = frame * 15
        c.fill_rect(0, fy, 68, 15, (22, 24, 28))
        c.bevel_sunken(0, fy, 68, 14, shadow=(10, 12, 14), light=(55, 60, 70))

        # Wedge bar graph: 28 levels
        # Active bars up to frame
        num_bars = frame
        for b in range(27):
            bx = 8 + b * 2
            # Height grows from 2 px on left to 8 px on right
            bh = 2 + int((b / 27.0) * 6)
            by = fy + 10 - bh
            if b <= num_bars:
                # Color gradient: green (b=0..16) -> yellow (b=17..22) -> red (b=23..27)
                if b < 17:
                    col = (0, 240, 50)
                elif b < 23:
                    col = (250, 220, 0)
                else:
                    col = (255, 40, 30)
            else:
                col = (28, 35, 30) # Unlit bar
            c.fill_rect(bx, by, 1, bh, col)

    # Thumbs at y = 420 (height 11, width 14):
    # Selected at (0, 420, 14, 11), Resting at (15, 420, 14, 11)
    for tx, is_sel in [(15, False), (0, True)]:
        bg = (70, 75, 85) if not is_sel else (55, 60, 70)
        c.fill_rect(tx, 420, 14, 11, bg)
        if is_sel:
            c.bevel_sunken(tx, 420, 14, 11, shadow=(20, 22, 28), light=(110, 120, 135))
        else:
            c.bevel_raised(tx, 420, 14, 11, light=(130, 140, 155), shadow=(25, 28, 35))
        # Center indicator notch
        notch_col = (0, 255, 60) if is_sel else (30, 35, 40)
        c.vline(tx + 6, 422, 7, notch_col)
        c.vline(tx + 7, 422, 7, (120, 130, 145))

    return c.to_bmp()

def generate_balance():
    """Generates balance.bmp (47 x 433). 28 frames of 15 rows each + thumb strip at y=420."""
    c = Canvas(47, 433, bg_color=(15, 16, 20))

    # 28 frames of balance track at (9, 0, 38, 420)
    for frame in range(28):
        fy = frame * 15
        c.fill_rect(9, fy, 38, 14, (22, 24, 28))
        c.bevel_sunken(9, fy, 38, 14, shadow=(10, 12, 14), light=(55, 60, 70))
        # Center mark at 9+19 = 28
        c.vline(28, fy + 2, 10, (45, 50, 60))

        # Balance indicator (frame 14 = center)
        pan_offset = frame - 14 # -14 to +13
        bar_x = 28 + int((pan_offset / 14.0) * 14)
        c.fill_rect(max(11, min(43, bar_x - 1)), fy + 4, 3, 6, (0, 240, 50))

    # Thumbs at y = 420 (height 11, width 14):
    # Selected/Active at (0, 420, 14, 11), Resting at (15, 420, 14, 11)
    for tx, is_sel in [(15, False), (0, True)]:
        bg = (70, 75, 85) if not is_sel else (55, 60, 70)
        c.fill_rect(tx, 420, 14, 11, bg)
        if is_sel:
            c.bevel_sunken(tx, 420, 14, 11, shadow=(20, 22, 28), light=(110, 120, 135))
        else:
            c.bevel_raised(tx, 420, 14, 11, light=(130, 140, 155), shadow=(25, 28, 35))
        notch_col = (0, 255, 60) if is_sel else (30, 35, 40)
        c.vline(tx + 6, 422, 7, notch_col)
        c.vline(tx + 7, 422, 7, (120, 130, 145))

    return c.to_bmp()

def generate_monoster():
    """Generates monoster.bmp (56 x 24)."""
    c = Canvas(56, 24, bg_color=(5, 6, 8))

    # Stereo:
    # Selected/Lit at (0, 0, 29, 12)
    c.fill_rect(0, 0, 29, 12, (5, 6, 8))
    draw_string(c, "stereo", 0, 3, fg=(0, 255, 60), bg=(5, 6, 8))
    # Dim at (0, 12, 29, 12)
    c.fill_rect(0, 12, 29, 12, (5, 6, 8))
    draw_string(c, "stereo", 0, 15, fg=(20, 50, 25), bg=(5, 6, 8))

    # Mono:
    # Selected/Lit at (29, 0, 27, 12)
    c.fill_rect(29, 0, 27, 12, (5, 6, 8))
    draw_string(c, "mono", 32, 3, fg=(0, 255, 60), bg=(5, 6, 8))
    # Dim at (29, 12, 27, 12)
    c.fill_rect(29, 12, 27, 12, (5, 6, 8))
    draw_string(c, "mono", 32, 15, fg=(20, 50, 25), bg=(5, 6, 8))

    return c.to_bmp()

def generate_playpaus():
    """Generates playpaus.bmp (42 x 9)."""
    c = Canvas(42, 9, bg_color=(5, 6, 8))

    # Playing indicator (0, 0, 9, 9): green triangle
    for col in range(6):
        c.vline(2 + col, 2 + col // 2, 5 - (col // 2) * 2, (0, 255, 60))

    # Paused indicator (9, 0, 9, 9): amber bars
    c.fill_rect(11, 2, 2, 5, (250, 200, 0))
    c.fill_rect(15, 2, 2, 5, (250, 200, 0))

    # Stopped indicator (18, 0, 9, 9): red square
    c.fill_rect(20, 2, 5, 5, (230, 40, 30))

    # Decoder working indicator at (36, 0, 3, 9): bright cyan/green
    c.fill_rect(36, 0, 3, 9, (0, 255, 200))

    # Decoder idle indicator at (39, 0, 3, 9): dark grey
    c.fill_rect(39, 0, 3, 9, (25, 30, 35))

    return c.to_bmp()

def generate_numbers():
    """Generates numbers.bmp (99 x 13). 11 cells of 9x13."""
    c = Canvas(99, 13, bg_color=(5, 8, 8))

    for digit in range(10):
        draw_7segment_cell(c, str(digit), digit * 9, 0)

    # 11th cell is NO_DIGIT (blank) at (90, 0)
    draw_7segment_cell(c, ' ', 90, 0)

    # Minus sign at (20, 6, 5, 1) solid green
    c.fill_rect(20, 6, 5, 1, (0, 255, 60))
    # No minus sign at (9, 6, 5, 1) dim
    c.fill_rect(9, 6, 5, 1, (12, 35, 18))

    return c.to_bmp()

def generate_nums_ex():
    """Generates nums_ex.bmp (108 x 13). 12 cells of 9x13."""
    c = Canvas(108, 13, bg_color=(5, 8, 8))

    for digit in range(10):
        draw_7segment_cell(c, str(digit), digit * 9, 0)

    # Cell 10 at (90, 0): NO_MINUS_SIGN_EX (blank)
    draw_7segment_cell(c, ' ', 90, 0)

    # Cell 11 at (99, 0): MINUS_SIGN_EX
    draw_7segment_cell(c, ' ', 99, 0)
    c.fill_rect(99 + 2, 6, 5, 1, (0, 255, 60))

    return c.to_bmp()

def generate_text():
    """Generates text.bmp (155 x 74). 4 copies of 31x3 grid of 5x6 cells."""
    c = Canvas(155, 74, bg_color=(0, 0, 0))

    row0 = list("abcdefghijklmnopqrstuvwxyz\"@  ")
    row1 = list("0123456789….:()-'!_+\\/[]^&%,=$#")
    row2 = ["Å", "Ö", "Ä", "?", "*"]

    # 4 copies at y = 0, 18, 38, 56
    color_schemes = [
        ((0, 255, 60), (0, 0, 0)),        # y = 0: classic green on black
        ((220, 240, 255), (25, 30, 40)),   # y = 18: white/cyan on dark slate
        ((0, 255, 60), (0, 0, 0)),        # y = 38: green on black
        ((240, 240, 240), (0, 0, 0)),      # y = 56: white on black
    ]

    for copy_idx, (base_y, (fg, bg)) in enumerate(zip([0, 18, 38, 56], color_schemes)):
        # Row 0
        for col_idx, ch in enumerate(row0):
            if col_idx < 31:
                draw_text_glyph(c, ch, col_idx * 5, base_y + 0, fg, bg)
        # Row 1
        for col_idx, ch in enumerate(row1):
            if col_idx < 31:
                draw_text_glyph(c, ch, col_idx * 5, base_y + 6, fg, bg)
        # Row 2
        for col_idx, ch in enumerate(row2):
            if col_idx < 31:
                draw_text_glyph(c, ch, col_idx * 5, base_y + 12, fg, bg)

    return c.to_bmp()

def generate_pledit():
    """Generates pledit.bmp (280 x 186)."""
    c = Canvas(280, 186, bg_color=(15, 16, 20))

    # Title bars (focused at y=0, unfocused at y=21):
    # Top left corner (0, y, 25, 20)
    # Title strip (26, y, 100, 20)
    # Top tile (127, y, 25, 20)
    # Top right corner (153, y, 25, 20)
    for focused, base_y in [(True, 0), (False, 21)]:
        bg = (20, 35, 80) if focused else (40, 44, 50)
        fg = (220, 235, 255) if focused else (150, 155, 165)
        # Corners & tiles
        c.fill_rect(0, base_y, 25, 20, bg)
        c.bevel_raised(0, base_y, 25, 20, light=(90, 110, 180) if focused else (75, 80, 90), shadow=(10, 15, 30))

        c.fill_rect(26, base_y, 100, 20, bg)
        c.bevel_raised(26, base_y, 100, 20, light=(90, 110, 180) if focused else (75, 80, 90), shadow=(10, 15, 30))
        draw_string(c, "playlist", 45, base_y + 7, fg=fg, bg=bg)

        c.fill_rect(127, base_y, 25, 20, bg)
        c.bevel_raised(127, base_y, 25, 20, light=(90, 110, 180) if focused else (75, 80, 90), shadow=(10, 15, 30))

        c.fill_rect(153, base_y, 25, 20, bg)
        c.bevel_raised(153, base_y, 25, 20, light=(90, 110, 180) if focused else (75, 80, 90), shadow=(10, 15, 30))

    # Side rails:
    # Left rail at (0, 42, 12, 29)
    c.fill_rect(0, 42, 12, 29, (35, 38, 45))
    c.bevel_raised(0, 42, 12, 29, light=(80, 85, 95), shadow=(15, 16, 20))

    # Right rail / scrollbar track at (31, 42, 20, 29)
    c.fill_rect(31, 42, 20, 29, (22, 24, 28))
    c.bevel_sunken(31, 42, 20, 29, shadow=(10, 12, 15), light=(60, 65, 75))

    # Scroll handle at (52, 53, 8, 18), selected at (61, 53, 8, 18)
    c.fill_rect(52, 53, 8, 18, (65, 70, 80))
    c.bevel_raised(52, 53, 8, 18, light=(120, 130, 145), shadow=(25, 28, 35))
    c.fill_rect(61, 53, 8, 18, (55, 60, 70))
    c.bevel_sunken(61, 53, 8, 18, shadow=(20, 22, 28), light=(100, 110, 125))

    # Scroll arrows at (261, 72, 8, 10)
    c.fill_rect(261, 72, 8, 10, (55, 60, 68))
    c.bevel_raised(261, 72, 8, 10, light=(100, 110, 125), shadow=(18, 20, 24))
    # Up arrow at (264, 73), down arrow at (264, 78)
    c.set(264, 74, (200, 215, 230))
    c.hline(263, 75, 3, (200, 215, 230))
    c.hline(263, 78, 3, (200, 215, 230))
    c.set(264, 79, (200, 215, 230))

    # Bottom bar:
    # Left block at (0, 72, 125, 38)
    c.fill_rect(0, 72, 125, 38, (38, 40, 48))
    c.bevel_raised(0, 72, 125, 38, light=(85, 90, 100), shadow=(15, 16, 20))
    # Right block at (126, 72, 150, 38)
    c.fill_rect(126, 72, 150, 38, (38, 40, 48))
    c.bevel_raised(126, 72, 150, 38, light=(85, 90, 100), shadow=(15, 16, 20))

    # Bottom buttons from sheet y = 86:
    # ADD (11), REM (43), SEL (72), MISC (101), LIST OPTS (229)
    for bx, label in [(11, "add"), (43, "rem"), (72, "sel"), (101, "misc"), (229, "opts")]:
        c.fill_rect(bx, 86, 24, 18, (55, 60, 68))
        c.bevel_raised(bx, 86, 24, 18, light=(95, 100, 115), shadow=(18, 20, 24))
        draw_string(c, label, bx + 2, 92, fg=(220, 230, 245), bg=(55, 60, 68))

    # Visualizer background at (205, 0, 75, 38)
    c.fill_rect(205, 0, 75, 38, (5, 6, 8))
    c.bevel_sunken(205, 0, 75, 38, shadow=(10, 12, 15), light=(60, 65, 75))

    return c.to_bmp()

def generate_eqmain():
    """Generates eqmain.bmp (275 x 315)."""
    c = Canvas(275, 315, bg_color=(20, 22, 26))

    # Window background at (0, 0, 275, 116)
    c.texture_metal(0, 0, 275, 116, base_color=(40, 42, 48))
    c.bevel_raised(0, 0, 275, 116, light=(85, 90, 100), shadow=(15, 16, 20))

    # Printed dB labels on main window: +12db at (40, 39), 0db at (44, 68), -12db at (40, 98)
    draw_string(c, "+12db", 42, 38, fg=(120, 130, 140), bg=(40, 42, 48))
    draw_string(c, " 0db", 42, 63, fg=(120, 130, 140), bg=(40, 42, 48))
    draw_string(c, "-12db", 42, 88, fg=(120, 130, 140), bg=(40, 42, 48))

    # Printed frequency band labels
    band_labels = ["60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"]
    for i, lbl in enumerate(band_labels):
        bx = 79 + i * 18 - 4
        draw_string(c, lbl, bx, 102, fg=(110, 120, 135), bg=(40, 42, 48))

    # Preamp label at (20, 102)
    draw_string(c, "pre", 20, 102, fg=(110, 120, 135), bg=(40, 42, 48))

    # Slider grooves under preamp (22) and bands (79 + 18k)
    for k in [-1] + list(range(10)):
        gx = 22 if k == -1 else 79 + k * 18
        c.fill_rect(gx + 4, 38, 3, 55, (12, 14, 16))
        c.bevel_sunken(gx + 4, 38, 3, 55, shadow=(8, 8, 10), light=(60, 65, 75))

    # Title bars:
    # Focused at (0, 134, 275, 14)
    c.fill_rect(0, 134, 275, 14, (20, 35, 80))
    c.bevel_raised(0, 134, 275, 14, light=(90, 110, 180), shadow=(10, 15, 30))
    draw_string(c, "equalizer", 80, 138, fg=(220, 235, 255), bg=(20, 35, 80))

    # Unfocused at (0, 149, 275, 14)
    c.fill_rect(0, 149, 275, 14, (45, 48, 55))
    c.bevel_raised(0, 149, 275, 14, light=(80, 85, 95), shadow=(18, 20, 24))
    draw_string(c, "equalizer", 80, 153, fg=(150, 155, 165), bg=(45, 48, 55))

    # Slider track area at (13, 164, 209, 129)
    c.fill_rect(13, 164, 209, 129, (25, 27, 32))

    # Slider thumbs:
    # Resting at (0, 164, 11, 11)
    c.fill_rect(0, 164, 11, 11, (75, 80, 90))
    c.bevel_raised(0, 164, 11, 11, light=(130, 140, 155), shadow=(25, 28, 35))
    c.hline(2, 169, 7, (30, 35, 40))
    c.hline(2, 170, 7, (120, 130, 145))

    # Dragged at (0, 176, 11, 11)
    c.fill_rect(0, 176, 11, 11, (65, 70, 80))
    c.bevel_sunken(0, 176, 11, 11, shadow=(20, 22, 28), light=(110, 120, 135))
    c.hline(2, 181, 7, (0, 255, 60))

    # On button: (10, 119, 26, 12) normal, (69, 119) engaged, (128, 119) depressed, (187, 119) engaged+depressed
    for bx, is_eng, is_dep in [(10, False, False), (69, True, False), (128, False, True), (187, True, True)]:
        bg = (42, 46, 52) if is_dep else (55, 60, 68)
        c.fill_rect(bx, 119, 26, 12, bg)
        if is_dep:
            c.bevel_sunken(bx, 119, 26, 12, shadow=(15, 16, 20), light=(80, 85, 95))
        else:
            c.bevel_raised(bx, 119, 26, 12, light=(95, 100, 115), shadow=(18, 20, 24))
        # Indicator dot
        c.fill_rect(bx + 2, 123, 3, 3, (0, 255, 60) if is_eng else (20, 50, 25))
        draw_string(c, "on", bx + 8, 122, fg=(220, 230, 245), bg=bg)

    # Auto button: (36, 119, 32, 12)
    for bx, is_eng, is_dep in [(36, False, False), (95, True, False), (154, False, True), (213, True, True)]:
        bg = (42, 46, 52) if is_dep else (55, 60, 68)
        c.fill_rect(bx, 119, 32, 12, bg)
        if is_dep:
            c.bevel_sunken(bx, 119, 32, 12, shadow=(15, 16, 20), light=(80, 85, 95))
        else:
            c.bevel_raised(bx, 119, 32, 12, light=(95, 100, 115), shadow=(18, 20, 24))
        c.fill_rect(bx + 2, 123, 3, 3, (0, 255, 60) if is_eng else (20, 50, 25))
        draw_string(c, "auto", bx + 7, 122, fg=(220, 230, 245), bg=bg)

    # Presets button: (224, 164, 44, 12) normal, (224, 176, 44, 12) selected
    for by, is_sel in [(164, False), (176, True)]:
        bg = (45, 50, 58) if is_sel else (55, 60, 68)
        c.fill_rect(224, by, 44, 12, bg)
        if is_sel:
            c.bevel_sunken(224, by, 44, 12, shadow=(15, 16, 20), light=(80, 85, 95))
        else:
            c.bevel_raised(224, by, 44, 12, light=(95, 100, 115), shadow=(18, 20, 24))
        draw_string(c, "presets", 227, by + 3, fg=(220, 230, 245), bg=bg)

    # Graph background at (0, 294, 113, 19)
    c.fill_rect(0, 294, 113, 19, (6, 8, 10))
    c.bevel_sunken(0, 294, 113, 19, shadow=(4, 5, 6), light=(50, 55, 65))
    # Horizontal grid line at y=303
    c.hline(2, 303, 109, (20, 35, 25))

    # Graph line color scale at (115, 294, 1, 19): 19 rows from red to green
    for y in range(19):
        t = y / 18.0
        # Row 0 = red, row 9 = green/yellow, row 18 = bright green
        r = int(255 * (1.0 - t))
        g = int(255 * min(1.0, t * 1.5))
        b = 30
        c.set(115, 294 + y, (r, g, b))

    # Preamp line at (0, 314, 113, 1)
    c.hline(0, 314, 113, (0, 255, 60))

    return c.to_bmp()

def generate_eq_ex():
    """Generates eq_ex.bmp (275 x 56)."""
    c = Canvas(275, 56, bg_color=(20, 22, 26))

    # Shade backgrounds:
    # Selected/Focused at (0, 0, 275, 14)
    c.fill_rect(0, 0, 275, 14, (20, 35, 80))
    c.bevel_raised(0, 0, 275, 14, light=(90, 110, 180), shadow=(10, 15, 30))
    draw_string(c, "equalizer", 80, 4, fg=(220, 235, 255), bg=(20, 35, 80))

    # Normal/Unfocused at (0, 15, 275, 14)
    c.fill_rect(0, 15, 275, 14, (45, 48, 55))
    c.bevel_raised(0, 15, 275, 14, light=(80, 85, 95), shadow=(18, 20, 24))
    draw_string(c, "equalizer", 80, 19, fg=(150, 155, 165), bg=(45, 48, 55))

    # Mini volume / balance slider thumbs at y = 30
    c.fill_rect(1, 30, 9, 7, (75, 80, 95))
    c.fill_rect(11, 30, 9, 7, (75, 80, 95))

    # Buttons at y = 38..56
    for by in [38, 47]:
        c.fill_rect(1, by, 9, 9, (55, 60, 68))
        c.fill_rect(11, by, 9, 9, (55, 60, 68))

    return c.to_bmp()

def generate_gen():
    """Generates gen.bmp (178 x 86)."""
    c = Canvas(178, 86, bg_color=(20, 22, 26))

    # Top border tiles (selected y=0, normal y=21)
    for base_y, focused in [(0, True), (21, False)]:
        bg = (20, 35, 80) if focused else (45, 48, 55)
        # Corners and fills
        c.fill_rect(0, base_y, 25, 20, bg)
        c.bevel_raised(0, base_y, 25, 20, light=(90, 110, 180) if focused else (80, 85, 95), shadow=(10, 15, 30))
        c.fill_rect(26, base_y, 25, 20, bg)
        c.fill_rect(52, base_y, 25, 20, bg)
        c.fill_rect(78, base_y, 25, 20, bg)
        c.fill_rect(104, base_y, 25, 20, bg)
        c.fill_rect(130, base_y, 25, 20, bg)

    # Bottom bar at (0, 42, 125, 14) and (0, 57, 125, 14)
    c.fill_rect(0, 42, 125, 14, (38, 40, 48))
    c.bevel_raised(0, 42, 125, 14, light=(85, 90, 100), shadow=(15, 16, 20))
    c.fill_rect(0, 57, 125, 14, (38, 40, 48))
    c.bevel_raised(0, 57, 125, 14, light=(85, 90, 100), shadow=(15, 16, 20))

    # Middle rails at (127, 42, 11, 29) and (139, 42, 8, 29)
    c.fill_rect(127, 42, 11, 29, (35, 38, 45))
    c.fill_rect(139, 42, 8, 29, (22, 24, 28))

    return c.to_bmp()


SHEET_GENERATORS = {
    "main.bmp": generate_main,
    "titlebar.bmp": generate_titlebar,
    "cbuttons.bmp": generate_cbuttons,
    "shufrep.bmp": generate_shufrep,
    "posbar.bmp": generate_posbar,
    "volume.bmp": generate_volume,
    "balance.bmp": generate_balance,
    "monoster.bmp": generate_monoster,
    "playpaus.bmp": generate_playpaus,
    "numbers.bmp": generate_numbers,
    "nums_ex.bmp": generate_nums_ex,
    "text.bmp": generate_text,
    "pledit.bmp": generate_pledit,
    "eqmain.bmp": generate_eqmain,
    "eq_ex.bmp": generate_eq_ex,
    "gen.bmp": generate_gen,
}

def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    print(f"Generating lookalike skin sheets to {OUTPUT_DIR}...")
    for filename, gen_fn in SHEET_GENERATORS.items():
        out_path = os.path.join(OUTPUT_DIR, filename)
        bmp_data = gen_fn()
        with open(out_path, "wb") as f:
            f.write(bmp_data)
        print(f"  {filename:15} -> {len(bmp_data):6} bytes")
    print("Done! All 16 skin sheets generated successfully.")

if __name__ == "__main__":
    main()
