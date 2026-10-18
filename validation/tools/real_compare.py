"""Compare emulator pictures with the Shaker site's real-CPC photos (the ground truth).

  real_compare.py PHOTO.webp PICTURE.png [PICTURE2 ...]      one screen
  real_compare.py --all TAG                                  every screen of $SHAKER_WORK/res_<M>_<TAG><crtc>

Photos: https://shaker.logonsystem.eu/images/cpc/CPC/<test>_CRTC<n>_[<subfolder>_]<subtest>.webp, kept in
$SHAKER_WORK/ref/real (fetch them with --fetch). --all also scores the AMSpiriT Lite references
($SHAKER_WORK/ref/ams_lite, ref/site) and writes $SHAKER_WORK/realcmp_<TAG>.json; it lists, per CRTC, the
screens where ours is clearly (0.1) further from the real CPC than the best AMSpiriT. Needs OpenCV
(pip install --user opencv-python-headless).

The photo is a camera shot of a CRT (perspective, curvature, glare): each emulator picture is
registered onto it (ECC, homography, on blurred grey levels) and scored by the correlation of the
two after registration (1 = same structure). Pictures: ours (272 lines, upscaled x2) and any
542/544-line reference.
"""
import cv2, numpy as np, os, sys

W, H = 620, 420  # working size of the photo


def load_photo(path):
    im = cv2.imread(path, cv2.IMREAD_GRAYSCALE)
    return cv2.resize(im, (W, H), interpolation=cv2.INTER_AREA).astype(np.float32) / 255


def load_emu(path):
    im = cv2.imread(path, cv2.IMREAD_GRAYSCALE)
    if im.shape[0] < 400:
        im = cv2.resize(im, (im.shape[1], im.shape[0] * 2), interpolation=cv2.INTER_NEAREST)
    return im.astype(np.float32) / 255


def prep(a):
    a = cv2.GaussianBlur(a, (0, 0), 1.5)
    return (a - a.mean()) / (a.std() + 1e-6)


def _ecc(e, p, init):
    warp = init.astype(np.float32)
    for scale in (0.25, 0.5, 1.0):
        ps = cv2.resize(p, None, fx=scale, fy=scale)
        S = np.diag([1 / scale, 1 / scale, 1]).astype(np.float32)
        Si = np.diag([scale, scale, 1]).astype(np.float32)
        _, w = cv2.findTransformECC(ps, e, (warp @ S).astype(np.float32), cv2.MOTION_HOMOGRAPHY,
                                    (cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_COUNT, 100, 1e-5), None, 5)
        warp = w @ Si
    return warp


def register(emu, photo):
    """Homography from the photo's frame into the emulator picture, and the warped picture: several
    framings are tried (the photos show the paper area only, or the whole tube), the best kept."""
    p, e = prep(photo), prep(emu)
    eh, ew = emu.shape
    best = None
    # emulator rectangle seen by the whole photo frame
    for x0, y0, x1, y1 in ((40, 50, 728, 520), (0, 0, ew, eh), (-60, -40, ew + 60, eh + 40), (-110, -70, ew + 110, eh + 70)):
        init = cv2.getPerspectiveTransform(
            np.float32([[0, 0], [W, 0], [W, H], [0, H]]), np.float32([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]))
        try:
            warp = _ecc(e, p, init)
        except cv2.error:
            warp = init
        out = cv2.warpPerspective(emu, warp, (W, H), flags=cv2.INTER_LINEAR + cv2.WARP_INVERSE_MAP)
        s = score(out, photo)
        if best is None or s > best[0]:
            best = (s, warp, out)
    return best[1], best[2]


def score(warped, photo):
    a, b = prep(warped), prep(photo)
    m = 10  # ignore the frame edges
    return float((a[m:-m, m:-m] * b[m:-m, m:-m]).mean())


WORK = os.environ.get("SHAKER_WORK", os.path.expanduser("~/shaker_work"))
MANIFEST = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "Shaker_CSL", "shakerland_manifest.json")


def screens():
    import json
    for t in json.load(open(MANIFEST))["tests"]:
        for s in t["subtests"]:
            for c in s["crtcs"]:
                yield f"{t['id']}_CRTC{c}_" + (s["subfolder"] + "_" if s.get("subfolder") else "") + s["subTest"], t["module"], c


def fetch():
    import urllib.request
    os.makedirs(f"{WORK}/ref/real", exist_ok=True)
    for n, _, _ in screens():
        dest = f"{WORK}/ref/real/{n}.webp"
        if os.path.exists(dest):
            continue
        data = urllib.request.urlopen(f"https://shaker.logonsystem.eu/images/cpc/CPC/{n}.webp", timeout=30).read()
        if data[:4] == b"RIFF":  # the site answers its HTML page for a missing picture
            open(dest, "wb").write(data)


def _one(job):
    n, m, c, tag = job
    photo = f"{WORK}/ref/real/{n}.webp"
    if not os.path.exists(photo):
        return None
    ph = load_photo(photo)
    out = {"name": n}
    for key, path in (("ours", f"{WORK}/res_{m}_{tag}{c}/{m}/{n}.png"), ("ams116", f"{WORK}/ref/ams_lite/{n}.png"),
                      ("ams115", f"{WORK}/ref/site/{n}.webp")):
        if os.path.exists(path):
            try:
                out[key] = score(register(load_emu(path), ph)[1], ph)
            except cv2.error:
                out[key] = None
    return out


def compare_all(tag):
    import json, collections
    from multiprocessing import Pool
    cv2.setNumThreads(1)
    with Pool(16) as pool:
        res = [r for r in pool.map(_one, [(n, m, c, tag) for n, m, c in screens()], chunksize=4) if r]
    json.dump(res, open(f"{WORK}/realcmp_{tag}.json", "w"), indent=0)
    for c in range(5):
        rows = [x for x in res if f"_CRTC{c}_" in x["name"] and x.get("ours") is not None]
        worse = []
        for x in rows:
            refs = [x[k] for k in ("ams116", "ams115") if x.get(k) is not None]
            if refs and x["ours"] < max(refs) - 0.1:
                worse.append(x["name"])
        tests = collections.Counter(n.split("_")[0] for n in worse)
        print(f"CRTC {c}: {len(rows)} screens, ours clearly further from the real CPC on {len(worse)}: "
              + ", ".join(f"{k}({v})" for k, v in tests.most_common()))


if __name__ == "__main__":
    if sys.argv[1] == "--fetch":
        fetch()
        sys.exit()
    if sys.argv[1] == "--all":
        compare_all(sys.argv[2])
        sys.exit()
    photo = load_photo(sys.argv[1])
    for p in sys.argv[2:]:
        emu = load_emu(p)
        warp, out = register(emu, photo)
        print(os.path.basename(p), round(score(out, photo), 3))
