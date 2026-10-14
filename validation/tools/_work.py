"""Shared paths for the validation tools.

Everything that is downloaded or generated lives in a work directory, kept out
of the repository: `$SHAKER_WORK` (default `~/shaker_work`) holds
  bundle/    the Shaker CSL scripts with their `csl_load` lines removed
  ref/ams/   the AMSpiriT reference screenshots (CRTC 0) of the Shaker site
  tests.json the site's list of tests (/api/tests)
  accc.txt   `pdftotext -layout ACCC1.9-EN.pdf` (optional, for grepping the doc)
"""
import os
import pathlib

REPO = pathlib.Path(__file__).resolve().parents[2]
WORK = pathlib.Path(os.environ.get("SHAKER_WORK", os.path.expanduser("~/shaker_work")))
BUNDLE = WORK / os.environ.get("BUNDLE_DIR", "bundle")
REFS = WORK / "ref" / "ams"
TESTS = WORK / "tests.json"
