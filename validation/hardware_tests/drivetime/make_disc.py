"""Build drivetime.dsk: a DATA format disc (40 tracks, sectors &C1-&C9 interleaved as AMSDOS formats
them) with DTIME.BIN (from basm's dtime.raw) and the files it loads, placed one after the other."""
import struct

TRACKS, SECTORS = 40, 9
ORDER = [0xC1, 0xC6, 0xC2, 0xC7, 0xC3, 0xC8, 0xC4, 0xC9, 0xC5]   # the order on the track
data = [[bytearray([0xE5] * 512) for _ in range(SECTORS)] for _ in range(TRACKS)]

def sector(linear):
    """The sector buffer of 512-byte sector number `linear` counted from track 0, &C1."""
    return data[linear // SECTORS][linear % SECTORS]

directory = []
next_block = 2  # blocks 0 and 1: the directory

def add(name, payload, load=0x0400, execute=0x0400):
    global next_block
    stem, ext = name.split('.')
    header = bytearray(128)
    header[1:9] = stem.ljust(8).encode(); header[9:12] = ext.ljust(3).encode()
    header[18] = 2  # binary
    struct.pack_into('<H', header, 21, load)
    struct.pack_into('<H', header, 24, len(payload))
    struct.pack_into('<H', header, 26, execute)
    header[64:67] = struct.pack('<I', len(payload))[:3]
    struct.pack_into('<H', header, 67, sum(header[:67]) & 0xFFFF)
    content = bytes(header) + payload
    blocks = []
    for offset in range(0, len(content), 1024):
        chunk = content[offset:offset + 1024].ljust(1024, b'\x1a')
        b = next_block; next_block += 1; blocks.append(b)
        sector(2 * b)[:] = chunk[:512]; sector(2 * b + 1)[:] = chunk[512:]
    records = (len(content) + 127) // 128
    for extent, i in enumerate(range(0, len(blocks), 16)):
        part = blocks[i:i + 16]
        rc = min(128, records - extent * 128)
        entry = bytes([0]) + stem.ljust(8).encode() + ext.ljust(3).encode() + bytes([extent, 0, 0, rc]) + bytes(part).ljust(16, b'\0')
        directory.append(entry)
    assert next_block <= TRACKS * SECTORS // 2

add('DTIME.BIN', open('dtime.raw', 'rb').read(), 0xA000, 0xA000)
add('BIG.BIN', bytes([0x5A]) * 0x9000)
add('PAD1.BIN', bytes(0x9000))
add('PAD2.BIN', bytes(0x9000))
add('PAD3.BIN', bytes(0x8000))
add('FAR.BIN', bytes(0x400))
for i in range(8):
    add(f'SMALL{i}.BIN', bytes(0x400))

dir_bytes = b''.join(directory).ljust(2048, b'\xe5')
for i in range(4):
    sector(i)[:] = dir_bytes[512 * i:512 * (i + 1)]

# extended DSK
out = bytearray(b'EXTENDED CPC DSK File\r\nDisk-Info\r\n'.ljust(34) + b'drivetime'.ljust(14))
out += bytes([TRACKS, 1, 0, 0]) + bytes([(0x100 + SECTORS * 512) // 256] * TRACKS)
out = out.ljust(0x100, b'\0')
for t in range(TRACKS):
    info = bytearray(b'Track-Info\r\n'.ljust(16, b'\0'))
    info += bytes([t, 0, 0, 0, 2, SECTORS, 0x4E, 0xE5])
    for r in ORDER:
        info += bytes([t, 0, r, 2, 0, 0]) + struct.pack('<H', 512)
    out += info.ljust(0x100, b'\0')
    for r in ORDER:
        out += data[t][r - 0xC1]
open('drivetime.dsk', 'wb').write(out)
print(f'{next_block} blocks used; FAR.BIN on track {(2 * (next_block - 9)) // SECTORS}')
