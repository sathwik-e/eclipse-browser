import math
import struct
import zlib

def make_png(width, height):
    # RGBA image buffer
    raw_data = bytearray()
    
    cx, cy = width / 2.0, height / 2.0
    # Outer squircle radius
    squircle_r = width * 0.45
    # Eclipse planet radius
    planet_r = width * 0.28
    
    for y in range(height):
        raw_data.append(0) # Filter type 0 (None)
        for x in range(width):
            dx = x - cx
            dy = y - cy
            dist = math.hypot(dx, dy)
            
            # Superellipse / Squircle for macOS icon base
            # |x|^4 + |y|^4 <= r^4
            sq_dist = (abs(dx)**3.2 + abs(dy)**3.2)**(1.0 / 3.2)
            
            if sq_dist > squircle_r:
                # Fully transparent outside icon
                raw_data.extend((0, 0, 0, 0))
                continue
                
            # Base icon background gradient (Obsidian dark space)
            edge_factor = max(0.0, min(1.0, (squircle_r - sq_dist) / 4.0))
            bg_grad = (y / height)
            bg_r = int(12 + bg_grad * 8)
            bg_g = int(14 + bg_grad * 10)
            bg_b = int(22 + bg_grad * 14)
            alpha = int(255 * edge_factor)
            
            # Corona glow around the planet
            # Glow is prominent between planet_r and planet_r * 1.45
            if dist > planet_r - 2 and dist < planet_r * 1.45:
                corona_intensity = max(0.0, 1.0 - (dist - planet_r) / (planet_r * 0.45))
                # Corona bias towards top-right (lunar crescent flare)
                angle = math.atan2(dy, dx)
                angle_weight = (math.cos(angle - 0.785) + 1.0) / 2.0 # Peak at 45 degrees
                flare = corona_intensity * (0.6 + 0.6 * angle_weight)
                
                c_r = int(245 * flare)
                c_g = int(158 * flare)
                c_b = int(11 * flare)
                
                # Blend with background
                bg_r = min(255, bg_r + c_r)
                bg_g = min(255, bg_g + c_g)
                bg_b = min(255, bg_b + c_b)
                
            # Planet disc (Obsidian core with subtle rim light)
            if dist <= planet_r:
                p_factor = dist / planet_r
                # Rim highlight
                rim = math.pow(p_factor, 12) * 0.7
                bg_r = int(8 + rim * 245)
                bg_g = int(9 + rim * 158)
                bg_b = int(14 + rim * 11)
            
            # Squircle border highlight
            border_w = 3.0
            if abs(sq_dist - squircle_r + border_w) < border_w:
                border_glow = (1.0 - abs(sq_dist - squircle_r + border_w) / border_w) * 0.35
                bg_r = min(255, int(bg_r + 255 * border_glow))
                bg_g = min(255, int(bg_g + 255 * border_glow))
                bg_b = min(255, int(bg_b + 255 * border_glow))
                
            raw_data.extend((bg_r, bg_g, bg_b, alpha))

    # Construct PNG
    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)

    png = b'\x89PNG\r\n\x1a\n'
    ihdr = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    png += chunk(b'IHDR', ihdr)
    compressed = zlib.compress(raw_data, 6)
    png += chunk(b'IDAT', compressed)
    png += chunk(b'IEND', b'')
    return png

if __name__ == '__main__':
    data = make_png(512, 512)
    with open('scripts/icon_512.png', 'wb') as f:
        f.write(data)
    print("Generated scripts/icon_512.png successfully")
