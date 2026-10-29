//! Realistic device identity generation, ported 1:1 from the proven
//! fqnovel-gonidbg implementation (internal/device/{models,generator}.go).
//!
//! Three independent pools are drawn per device — real `ro.product.model`
//! codes (never marketing names: those are a wind-control fingerprint),
//! Android version (with its EMUI-style release build), and resolution.
//! The register payload fields that reference them are assembled in
//! `register.rs`; the values here are byte-compatible with the reference.

use rand::Rng;

/// One real handset: brand + `ro.product.model` code. Marketing names are
/// deliberately NOT sent on the wire (see the reference's models.go).
pub struct ModelSpec {
    pub brand: &'static str,
    pub model: &'static str,
}

/// Real China-region model codes (source: KHwang9883/MobileModels,
/// cross-checked; 73 handsets).
static MODELS: &[ModelSpec] = &[
    ModelSpec {
        brand: "Xiaomi",
        model: "2201123C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2201122C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2206123SC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2206122SC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2211133C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2210132C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2304FPN6DC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "23127PN0CC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "23116PN5BC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "24031PN0DC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "22041211AC",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "22011211C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "23013RK75C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "22127RK46C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "23113RKC6C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "23117RK66C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "22101316C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "2312DRA50C",
    },
    ModelSpec {
        brand: "Xiaomi",
        model: "23090RA98C",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "TAS-AL00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "NOH-AN00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "OCE-AN10",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "CET-AL00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "DCO-AL00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "BRA-AL00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "ALN-AL00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "ANA-AN00",
    },
    ModelSpec {
        brand: "HUAWEI",
        model: "ELS-AN00",
    },
    ModelSpec {
        brand: "HONOR",
        model: "LGE-AN00",
    },
    ModelSpec {
        brand: "HONOR",
        model: "PGT-AN00",
    },
    ModelSpec {
        brand: "HONOR",
        model: "PGT-AN10",
    },
    ModelSpec {
        brand: "HONOR",
        model: "BVL-AN00",
    },
    ModelSpec {
        brand: "HONOR",
        model: "BVL-AN16",
    },
    ModelSpec {
        brand: "HONOR",
        model: "PTP-AN00",
    },
    ModelSpec {
        brand: "HONOR",
        model: "PTP-AN10",
    },
    ModelSpec {
        brand: "HONOR",
        model: "ANN-AN00",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PEEM00",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PFFM10",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PFEM10",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PGFM10",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PGEM10",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PHZ110",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PHY110",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PGBM10",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PHM110",
    },
    ModelSpec {
        brand: "OPPO",
        model: "PHW110",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2183A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2185A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2241A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2242A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2227A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2309A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2324A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2203A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2244A",
    },
    ModelSpec {
        brand: "vivo",
        model: "V2283A",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "LE2110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "LE2120",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "MT2110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "PHB110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "PJD110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "PJZ110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "PHK110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "PJA110",
    },
    ModelSpec {
        brand: "OnePlus",
        model: "PJE110",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-S9010",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-S9080",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-S9110",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-S9180",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-S9210",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-S9280",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-F9360",
    },
    ModelSpec {
        brand: "Samsung",
        model: "SM-F9460",
    },
];

/// Android version pool with the EMUI-style release build each pairs with
/// (the reference's androidVersions).
static ANDROID_VERSIONS: &[(u32, u32, &str)] = &[
    (12, 32, "V417IR"), // (os_version, os_api, release)
    (13, 33, "V433IR"),
    (11, 30, "V394IR"),
    (14, 34, "V451IR"),
];

/// Resolution pool: (resolution, dpi, density bucket).
static RESOLUTIONS: &[(&str, u32, &str)] = &[
    ("2400*1080", 480, "xxhdpi"),
    ("2340*1080", 440, "xxhdpi"),
    ("1920*1080", 480, "xxhdpi"),
    ("2560*1440", 560, "xxxhdpi"),
    ("3200*1440", 640, "xxxhdpi"),
];

/// ROM code pool (the register payload's `rom` field).
static ROM_CODES: &[&str] = &["1414", "1415", "1416", "1417", "1418", "1419", "1420"];

/// CPU ABI pool; arm64 dominates real traffic, armeabi stays for variety.
static CPU_ABIS: &[&str] = &["arm64-v8a", "armeabi-v7a"];

/// One drawn device surface (everything the register payload needs).
pub struct DeviceSurface {
    pub brand: String,
    pub manufacturer: String,
    pub model: String,
    pub os_version: String,
    pub os_api: u32,
    pub release_build: String,
    pub rom_version: String,
    pub rom: String,
    pub resolution: String,
    pub dpi: String,
    pub display_density: String,
    pub host_abi: String,
    pub cpu_abi: String,
    pub user_agent: String,
    /// Random 16-hex android_id.
    pub android_id: String,
    /// Random 40-hex OpenUDID.
    pub openudid: String,
    /// Random 32-hex sig_hash.
    pub sig_hash: String,
    /// Random IPv6 (register payload ipv6_list).
    pub ipv6_address: String,
    /// UUID request id.
    pub req_id: String,
    /// Fake "APK first installed" timestamp (1 day .. 1 year ago).
    pub apk_first_install_ms: i64,
}

/// Real `ro.product.manufacturer` for a brand (Samsung reports lowercase).
fn manufacturer_for(brand: &str) -> String {
    if brand == "Samsung" {
        "samsung".to_string()
    } else {
        brand.to_string()
    }
}

/// Draws a complete random device surface (uniform over each pool).
pub fn random_surface() -> DeviceSurface {
    let mut rng = rand::thread_rng();
    let m = &MODELS[rng.gen_range(0..MODELS.len())];
    let &(osv, api, release) = &ANDROID_VERSIONS[rng.gen_range(0..ANDROID_VERSIONS.len())];
    let &(res, dpi, density) = &RESOLUTIONS[rng.gen_range(0..RESOLUTIONS.len())];
    let rom = ROM_CODES[rng.gen_range(0..ROM_CODES.len())];
    let host_abi = CPU_ABIS[rng.gen_range(0..CPU_ABIS.len())];
    let cpu_abi = CPU_ABIS[rng.gen_range(0..CPU_ABIS.len())];
    let now_ms = crate::timeutil::now_millis();
    // 1 day .. 1 year ago.
    let install_ago = rng.gen_range(86_400_000..31_536_000_000);
    DeviceSurface {
        brand: m.brand.into(),
        manufacturer: manufacturer_for(m.brand),
        model: m.model.into(),
        os_version: osv.to_string(),
        os_api: api,
        release_build: format!("{release}_20171120"),
        rom_version: format!("{release}+release-keys"),
        rom: rom.into(),
        resolution: res.into(),
        dpi: dpi.to_string(),
        display_density: density.into(),
        host_abi: host_abi.into(),
        cpu_abi: cpu_abi.into(),
        user_agent: format!(
            "com.dragon.read.oversea.gp/68132 (Linux; U; Android {osv}; zh_CN; {}; Build/{release};tt-ok/3.12.13.4-tiktok)",
            m.model
        ),
        android_id: random_hex(16),
        openudid: random_hex(40),
        sig_hash: random_hex(32),
        ipv6_address: random_ipv6(&mut rng),
        req_id: crate::device::crypto::uuid_v4(),
        apk_first_install_ms: now_ms - install_ago,
    }
}

/// Random 16-hex device serial (the emulated signer's device_json format).
pub fn random_serial() -> String {
    random_hex(16)
}

fn random_hex(n: usize) -> String {
    const CHARS: &[u8] = b"0123456789abcdef";
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| CHARS[rng.gen_range(0..16)] as char)
        .collect()
}

fn random_ipv6(rng: &mut rand::rngs::ThreadRng) -> String {
    // The reference's randomIPv6: 2408:xxxx-style China carrier prefixes.
    fn seg(rng: &mut rand::rngs::ThreadRng) -> String {
        format!("{:x}", rng.gen_range(0..0x10000))
    }
    format!(
        "2408:{}:{}:{}:{}:{}:{}:{}",
        seg(rng),
        seg(rng),
        seg(rng),
        seg(rng),
        seg(rng),
        seg(rng),
        seg(rng)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_fields_are_well_formed() {
        for _ in 0..100 {
            let s = random_surface();
            assert!(!s.brand.is_empty() && !s.model.is_empty());
            assert!((30..=34).contains(&s.os_api));
            assert!(s.resolution.contains('*'));
            assert!(s.user_agent.contains(&s.model));
            assert_eq!(s.android_id.len(), 16);
            assert_eq!(s.openudid.len(), 40);
            assert_eq!(s.sig_hash.len(), 32);
            assert!(s.ipv6_address.starts_with("2408:"));
            assert!(s.rom_version.ends_with("+release-keys"));
            assert!(s.apk_first_install_ms > 0);
        }
    }

    #[test]
    fn random_surface_covers_models() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            seen.insert(random_surface().model);
        }
        // 73 models, 2000 draws: missing one is vanishingly unlikely.
        assert_eq!(seen.len(), MODELS.len());
    }
}
