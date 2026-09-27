#!/usr/bin/env python3
"""Generates tests/z80_reference/vectors.txt: single-instruction executions of a
well-tested Z80 core (kosarev's `z80` Python package, `pip install z80`), to
check ours against.

Every vector is one line:

    <instruction bytes> <seed> <state> <state> <writes>

where a <state> is 16 hexadecimal fields (before, then after):
AF BC DE HL AF' BC' DE' HL' IX IY SP PC I R IFF1 IFF2
and <writes> is `addr=value` pairs (the memory writes, in order).

The memory is not stored: byte `a` of a vector's memory is `memory_byte(a,
seed)` (also implemented by the Rust test) except for the instruction itself,
placed at PC.

Instructions doing I/O, halting or depending on interrupts are left out.
"""
import random
import sys

import z80

PC = 0x4000


def memory_byte(address, seed):
    return ((((address ^ seed) * 0x9E3779B1) & 0xFFFFFFFF) >> 13) & 0xFF


STATE = ['af', 'bc', 'de', 'hl', 'alt_af', 'alt_bc', 'alt_de', 'alt_hl', 'ix', 'iy', 'sp', 'pc', 'i', 'r']


def load(machine, seed, code, state):
    machine.memory[:] = bytes(memory_byte(a, seed) for a in range(0x10000))
    machine.memory[PC:PC + len(code)] = bytes(code)
    for name, value in zip(STATE, state):
        setattr(machine, name, value)
    machine.iff1 = bool(state[14])
    machine.iff2 = bool(state[15])


def snapshot(machine):
    return [getattr(machine, name) for name in STATE] + [int(machine.iff1), int(machine.iff2)]


def fmt(state):
    return ' '.join('%04x' % v if i < 12 else '%02x' % v for i, v in enumerate(state[:14])) + ' %d %d' % (state[14], state[15])


# opcodes that use HL, H, L, (HL) or IX-relevant stack/jump forms
USES_HL = (
    {0x09, 0x19, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E,
     0x34, 0x35, 0x36, 0x39, 0xE1, 0xE3, 0xE5, 0xE9, 0xF9}
    | {op for op in range(0x40, 0x80) if op != 0x76 and (op >> 3 & 7 in (4, 5, 6) or op & 7 in (4, 5, 6))}
    | {op for op in range(0x80, 0xC0) if op & 7 in (4, 5, 6)}
)


def opcodes():
    """(bytes without operands, number of operand bytes) for the instructions to test."""
    skipped_plain = {0x76, 0xDB, 0xD3, 0xF3, 0xFB, 0xCB, 0xDD, 0xED, 0xFD}
    for op in range(256):
        if op in skipped_plain:
            continue
        yield [op]
    for op in range(256):
        yield [0xCB, op]
    for op in range(256):
        # ED: only the documented + block instructions; I/O ones are left out
        if 0x40 <= op <= 0x7F and (op & 7) in (0, 1):
            continue
        if op in (0xA2, 0xA3, 0xAA, 0xAB, 0xB2, 0xB3, 0xBA, 0xBB):
            continue
        yield [0xED, op]
    for prefix in (0xDD, 0xFD):
        for op in range(256):
            # the prefix only means something to instructions using HL, H, L
            # or (HL); on the others it is a NOP (undocumented, not tested)
            if op not in USES_HL:
                continue
            yield [prefix, op]
        for op in range(256):
            # the forms copying the result into a register are undocumented
            if op & 7 == 6:
                yield [prefix, 0xCB, None, op]  # displacement is random


def main():
    per_instruction = int(sys.argv[1]) if len(sys.argv) > 1 else 6
    rng = random.Random(1985)
    machine = z80.Z80Machine()
    writes = []
    machine.set_write_callback(lambda address, value: writes.append((address, value)))
    out = sys.stdout

    for template in opcodes():
        for _ in range(per_instruction):
            seed = rng.getrandbits(32)
            code = [rng.getrandbits(8) if b is None else b for b in template]
            # operand bytes (immediate / displacement) after the opcode
            code += [rng.getrandbits(8) for _ in range(2)]
            if template[:2] in ([0xDD, 0xCB], [0xFD, 0xCB]):
                code = [template[0], 0xCB, code[2], template[3]] + [0, 0]
            state = [rng.getrandbits(16) for _ in range(11)] + [PC, rng.getrandbits(8), rng.getrandbits(8), 0, 0]
            # some states with special values catch corner cases the random ones miss
            special = rng.choice([None, None, 0x0000, 0xFFFF, 0x7FFF, 0x8000, 0x00FF, 0xFF00])
            if special is not None:
                for i in rng.sample(range(11), 3):
                    state[i] = special
            load(machine, seed, code, state)
            del writes[:]
            machine.ticks_to_stop = 1
            machine.run()
            if code[0] in (0xDD, 0xFD):
                # the reference executes the index prefix as an instruction of its own
                machine.ticks_to_stop = 1
                machine.run()
            after = snapshot(machine)
            out.write('%s %08x %s %s %s\n' % (
                ''.join('%02x' % b for b in code),
                seed, fmt(state), fmt(after), ' '.join('%04x=%02x' % w for w in writes)))


if __name__ == '__main__':
    main()
