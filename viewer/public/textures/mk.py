import struct, zlib, os

W, H = 2048, 1024
R, G, B = 34, 85, 170

# Build minimal JPEG using only stdlib
# We'll create a BMP instead (simpler) then note it for the user
# Actually let's create a PPM (simplest image format) and note it

out = '/home/sksat/prog/orts/viewer/public/textures/earth_blue_marble.jpg'

# Create a minimal valid JPEG
# JPEG with a single solid color block

import io

# Use a minimal approach: write PPM first (browsers won't read it)
# Better: write a minimal PNG using zlib from stdlib

def create_png(width, height, r, g, b, filename):
    def chunk(chunk_type, data):
        c = chunk_type + data
        crc = struct.pack('>I', zlib.crc32(c) & 0xffffffff)
        return struct.pack('>I', len(data)) + c + crc

    # Signature
    sig = b'\x89PNG\r\n\x1a\n'

    # IHDR
    ihdr_data = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
    ihdr = chunk(b'IHDR', ihdr_data)

    # IDAT - raw pixel data with filter bytes
    raw_row = b'\x00' + bytes([r, g, b]) * width
    raw_data = raw_row * height
    compressed = zlib.compress(raw_data)
    idat = chunk(b'IDAT', compressed)

    # IEND
    iend = chunk(b'IEND', b'')

    with open(filename, 'wb') as f:
        f.write(sig + ihdr + idat + iend)

# Save as PNG (rename to .png)
png_out = out.replace('.jpg', '.png')
create_png(W, H, R, G, B, png_out)
print(f'Created {png_out}')
sz = os.path.getsize(png_out)
print(f'Size: {sz} bytes ({sz/1024:.1f} KB)')
