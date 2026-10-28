//! Realistic device-model surface for registration.
//!
//! Every pooled device used to register as the same fixed "MI 12" surface —
//! a device-farm signature. [`random_model`] picks one entry of a table of
//! real handsets (brand, model code, os_api/os_version pair, resolution,
//! dpi, rom strings), so each registered device presents a coherent,
//! plausible identity. The chosen surface is persisted with the device
//! (see `Device` in `device/mod.rs`) and reused for its whole lifetime.

use rand::Rng;

/// One coherent device surface. Values mirror what the real app sends for
/// that handset: `model` is the `device_type` query param, `rom`/`rom_version`
/// follow each brand's format.
pub struct ModelSpec {
    pub brand: &'static str,
    pub model: &'static str,
    pub os_api: &'static str,
    pub os_version: &'static str,
    pub resolution: &'static str,
    pub dpi: &'static str,
    pub rom: &'static str,
    pub rom_version: &'static str,
}

/// Handsets common in the FQNovel user base (mainland market). os_api and
/// os_version pairs are what that model actually runs (or ran after vendor
/// updates); rom strings follow each brand's real format.
static MODELS: &[ModelSpec] = &[
    // HUAWEI / honor
    ModelSpec {
        brand: "HUAWEI",
        model: "ANA-AN00",
        os_api: "32",
        os_version: "13",
        resolution: "2340*1080",
        dpi: "480",
        rom: "EMUI",
        rom_version: "V417IR release-keys",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "ELS-AN00",
        os_api: "30",
        os_version: "11",
        resolution: "2340*1080",
        dpi: "480",
        rom: "EMUI",
        rom_version: "V417IR release-keys",
    },
    ModelSpec {
        brand: "honor",
        model: "FRD-AL10",
        os_api: "28",
        os_version: "9",
        resolution: "1080*1920",
        dpi: "480",
        rom: "EMUI",
        rom_version: "FRD-AL10+8.0.0.556(C00)",
    },
    ModelSpec {
        brand: "honor",
        model: "ANY-AN00",
        os_api: "30",
        os_version: "11",
        resolution: "2340*1080",
        dpi: "480",
        rom: "Magic",
        rom_version: "5.2.0.112(C00E112R5P5)",
    },
    // Xiaomi / Redmi
    ModelSpec {
        brand: "Xiaomi",
        model: "23127PN0CC",
        os_api: "34",
        os_version: "14",
        resolution: "1200*2670",
        dpi: "460",
        rom: "MIUI",
        rom_version: "V14.0.6.0.TMCCNXM",
    },
    ModelSpec {
        brand: "Redmi",
        model: "25053RT47C",
        os_api: "36",
        os_version: "16",
        resolution: "1280*2772",
        dpi: "520",
        rom: "HyperOS",
        rom_version: "OS3.0.4.0.VOLCNXM",
    },
    ModelSpec {
        brand: "Redmi",
        model: "M2012K11AC",
        os_api: "33",
        os_version: "13",
        resolution: "1080*2400",
        dpi: "440",
        rom: "MIUI",
        rom_version: "V14.0.6.0.TKCCNXM",
    },
    // OPPO
    ModelSpec {
        brand: "OPPO",
        model: "PGKM10",
        os_api: "33",
        os_version: "13",
        resolution: "1080*2412",
        dpi: "480",
        rom: "ColorOS",
        rom_version: "13.1",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PHK110",
        os_api: "34",
        os_version: "14",
        resolution: "1216*2672",
        dpi: "480",
        rom: "ColorOS",
        rom_version: "14.0",
    },
    // vivo
    ModelSpec {
        brand: "vivo",
        model: "V2312A",
        os_api: "34",
        os_version: "14",
        resolution: "1260*2800",
        dpi: "480",
        rom: "OriginOS",
        rom_version: "4",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2309A",
        os_api: "34",
        os_version: "14",
        resolution: "1080*2400",
        dpi: "440",
        rom: "OriginOS",
        rom_version: "4",
    },
    // samsung
    ModelSpec {
        brand: "samsung",
        model: "SM-S9110",
        os_api: "34",
        os_version: "14",
        resolution: "1080*2340",
        dpi: "425",
        rom: "OneUI",
        rom_version: "6.1",
    },
    ModelSpec {
        brand: "samsung",
        model: "SM-A5560",
        os_api: "34",
        os_version: "14",
        resolution: "1080*2340",
        dpi: "400",
        rom: "OneUI",
        rom_version: "6.1",
    },
    // Google
    ModelSpec {
        brand: "google",
        model: "Pixel 6",
        os_api: "33",
        os_version: "13",
        resolution: "1080*2400",
        dpi: "420",
        rom: "google",
        rom_version: "TQ3A.230805.001",
    },
    ModelSpec {
        brand: "google",
        model: "Pixel 8",
        os_api: "34",
        os_version: "14",
        resolution: "1080*2400",
        dpi: "420",
        rom: "google",
        rom_version: "UD1A.231105.004",
    },
];

/// Uniformly random model from the table.
pub fn random_model() -> &'static ModelSpec {
    &MODELS[rand::thread_rng().gen_range(0..MODELS.len())]
}

/// Random 16-lowercase-hex device serial (the format of the emulated
/// signer's base device; see fqsign.h `device_json`).
pub fn random_serial() -> String {
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| format!("{:x}", rng.gen_range(0..16)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_is_sixteen_lowercase_hex() {
        for _ in 0..100 {
            let s = random_serial();
            assert_eq!(s.len(), 16);
            assert!(s
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        }
    }

    #[test]
    fn models_are_self_consistent() {
        for m in MODELS {
            assert!(!m.brand.is_empty() && !m.model.is_empty());
            // os_api and os_version must be a plausible Android pair.
            let api: u32 = m.os_api.parse().unwrap();
            let ver: u32 = m.os_version.parse().unwrap();
            assert!((21..=36).contains(&api));
            assert!((5..=16).contains(&ver));
            assert!(m.resolution.contains('*'));
            let dpi: u32 = m.dpi.parse().unwrap();
            assert!((300..=600).contains(&dpi));
        }
    }

    #[test]
    fn random_model_covers_the_table() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..500 {
            seen.insert(random_model().model);
        }
        // With 16 entries and 500 draws, missing one is (15/16)^500 ≈ 1e-14.
        assert_eq!(seen.len(), MODELS.len());
    }
}
