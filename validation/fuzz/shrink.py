#!/usr/bin/env python3
"""Shrink a diverging fuzz case: shrink.py <seed> [events] [us]  (delta-debugging on the event list)."""
import ast, os, subprocess, sys
seed = sys.argv[1]; events = sys.argv[2] if len(sys.argv) > 2 else '30'; us = sys.argv[3] if len(sys.argv) > 3 else '80000'
here = os.path.dirname(os.path.abspath(__file__))
out = subprocess.run([sys.executable, os.path.join(here, 'gen_crtc_fuzz.py'), seed, '/tmp/shrink.sna', events],
                     capture_output=True, text=True, env=dict(os.environ, FUZZ_LOG='1')).stdout
ev = ast.literal_eval(out.split('):', 1)[1].split('\n')[0].strip())
def diverges(ev):
    subprocess.run([sys.executable, os.path.join(here, 'gen_crtc_fuzz.py'), seed, '/tmp/shrink.sna'], capture_output=True,
                   env=dict(os.environ, FUZZ_EVENTS=repr(ev)))
    r = subprocess.run([sys.executable, os.path.join(here, 'compare_lines.py'), '/tmp/shrink.sna', us], capture_output=True, text=True)
    return 'differs' in r.stdout
assert diverges(ev), 'does not diverge'
i = 0
while i < len(ev):
    t = ev[:i] + ev[i + 1:]
    if diverges(t): ev = t
    else: i += 1
print(ev)
