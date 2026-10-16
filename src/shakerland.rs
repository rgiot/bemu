//! The catalogue of tests published at <https://shaker.logonsystem.eu/tests>:
//! which SSM code (see `SSM-STANDARD-EN.pdf`) belongs to which named test and
//! sub-test, on which CRTC types. Fetched from the site's own `/api/tests`
//! and `/api/archs` and saved verbatim as `validation/Shaker_CSL/shakerland_manifest.json`,
//! so a screenshot can be named the way the site names its own
//! (`<test id>_CRTC<n>_<subtest>.png`) for a direct side-by-side comparison.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct RawManifest {
    tests: Vec<RawTest>
}

#[derive(Debug, Deserialize)]
struct RawTest {
    module: String,
    id: String,
    subtests: Vec<RawSubTest>
}

#[derive(Debug, Deserialize)]
struct RawSubTest {
    #[serde(rename = "subTest")]
    sub_test: String,
    hex: String,
    /// B1's sub-tests repeat their names in several R9 settings (`R9E7`, `R9E6`...).
    #[serde(default)]
    subfolder: Option<String>
}

/// One entry: `module` ('A'-'E') and the 16-bit SSM code both identify it.
pub struct TestId {
    pub test_id: String,
    pub sub_test: String,
    pub subfolder: Option<String>
}

/// Maps `(module, SSM code)` to the test/sub-test the site knows it as.
pub struct Manifest {
    by_code: HashMap<(char, u16), TestId>
}

impl Manifest {
    /// The manifest bundled with the repository (`validation/Shaker_CSL/shakerland_manifest.json`).
    pub fn bundled() -> Self {
        Self::parse(include_str!("../validation/Shaker_CSL/shakerland_manifest.json"))
            .expect("bundled shakerland_manifest.json must parse")
    }

    pub fn parse(json: &str) -> Result<Self, String> {
        let raw: RawManifest = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut by_code = HashMap::new();
        for test in raw.tests {
            let module = test
                .module
                .chars()
                .next()
                .ok_or_else(|| format!("test {} has an empty module", test.id))?;
            for sub in test.subtests {
                let code = u16::from_str_radix(&sub.hex, 16)
                    .map_err(|e| format!("{}/{}: bad hex {:?}: {e}", test.id, sub.sub_test, sub.hex))?;
                by_code.insert(
                    (module, code),
                    TestId {
                        test_id: test.id.clone(),
                        sub_test: sub.sub_test,
                        subfolder: sub.subfolder
                    }
                );
            }
        }
        Ok(Manifest { by_code })
    }

    /// The test/sub-test a `module`'s program identifies itself as when it
    /// raises SSM code `code`, if the manifest knows it.
    pub fn lookup(&self, module: char, code: u16) -> Option<&TestId> {
        self.by_code.get(&(module, code))
    }

    /// The filename (with no directory and no extension) the results site
    /// itself would use for this shot: `<test id>_CRTC<n>_<subtest>`, or a
    /// generic `<module><crtc>_<code>` when the manifest doesn't know it
    /// (e.g. module E, whose tests are not published there yet).
    pub fn file_stem(&self, module: char, crtc: u8, code: u16) -> String {
        match self.lookup(module, code) {
            Some(TestId { test_id, sub_test, subfolder: Some(folder) }) => format!("{test_id}_CRTC{crtc}_{folder}_{sub_test}"),
            Some(test) => format!("{}_CRTC{crtc}_{}", test.test_id, test.sub_test),
            None => format!("{module}_CRTC{crtc}_{code:04X}")
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_bundled_manifest_parses_and_finds_a_known_code() {
        let manifest = Manifest::bundled();
        // A1/A (see the manifest): "UPDATE VRAM VS CRTC/GA", sub-test A, code 0001
        let found = manifest.lookup('A', 0x0001).expect("A1/A must be in the manifest");
        assert_eq!(found.test_id, "A1");
        assert_eq!(found.sub_test, "A");
        assert_eq!(manifest.file_stem('A', 0, 0x0001), "A1_CRTC0_A");
    }

    /// B1 names its pictures after the R9 setting too, as the site does (`B1_CRTC1_R9E7_K1.webp`).
    #[test]
    fn a_subfolder_is_part_of_the_name() {
        let manifest = Manifest::bundled();
        assert_eq!(manifest.file_stem('B', 1, 0x03E5), "B1_CRTC1_R9E7_K1");
        assert_eq!(manifest.file_stem('B', 1, 0x03E7), "B1_CRTC1_R9E6_K1");
    }

    #[test]
    fn an_unknown_code_falls_back_to_a_generic_name() {
        let manifest = Manifest::bundled();
        assert!(manifest.lookup('E', 0x0001).is_none(), "module E isn't published there yet");
        assert_eq!(manifest.file_stem('E', 2, 0x1234), "E_CRTC2_1234");
    }
}
