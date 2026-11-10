import struct, sys
from PIL import Image
data = open(sys.argv[1], 'rb').read()
h = struct.unpack('>25I', data[:100])
hsize, ver, fmt, depth, w, ht, xoff, border, bunit, bbo, bpad, bpp, bpl = h[:13]
ncolors = h[19]
off = hsize + ncolors * 12
px = data[off:off + bpl * ht]
if bpp == 24:
    img = Image.frombuffer('RGB', (w, ht), px, 'raw', 'BGR' if border == 0 else 'RGB', bpl, 1)
else:
    img = Image.frombuffer('RGBX', (w, ht), px, 'raw', 'BGRX' if border == 0 else 'XRGB', bpl, 1).convert('RGB')
if len(sys.argv) > 3:
    cw, ch = map(int, sys.argv[3].split('x'))
    img = img.crop((0, 0, cw, ch))
img.save(sys.argv[2])
