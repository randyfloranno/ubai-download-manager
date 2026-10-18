//! Embeds the UDM logo as the `udm.exe` icon on Windows (MSVC).
//!
//! The `.res` file is written directly (RT_ICON + RT_GROUP_ICON) from
//! `udm_core::logo`, so no resource compiler or extra crate is needed.

use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const SIZES: [u16; 6] = [16, 24, 32, 48, 64, 256];

/// One `.res` entry with numeric type and name, data padded to 4 bytes.
fn push_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) -> Option<()> {
    out.extend_from_slice(&u32::try_from(data.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&32_u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, name] {
        out.extend_from_slice(&u16::to_le_bytes(v));
    }
    out.extend_from_slice(&0_u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0x0409_u16.to_le_bytes()); // en-US; icons are language neutral
    out.extend_from_slice(&[0; 8]); // Version, Characteristics
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    Some(())
}

fn res_file() -> Option<Vec<u8>> {
    let mut out = Vec::new();
    push_resource(&mut out, 0, 0, 0, &[])?; // mandatory empty first entry
    let count = u16::try_from(SIZES.len()).ok()?;
    let mut group = Vec::new();
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&count.to_le_bytes());
    for (id, &size) in (1_u16..).zip(SIZES.iter()) {
        let dib = udm_core::logo::dib(size)?;
        push_resource(&mut out, RT_ICON, id, 0x1010, &dib)?;
        let edge = u8::try_from(size).unwrap_or(0); // 0 means 256
        group.extend_from_slice(&[edge, edge, 0, 0]);
        group.extend_from_slice(&1_u16.to_le_bytes());
        group.extend_from_slice(&32_u16.to_le_bytes());
        group.extend_from_slice(&u32::try_from(dib.len()).ok()?.to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut out, RT_GROUP_ICON, 1, 0x1030, &group)?;
    Some(out)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|v| v == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|v| v == "msvc");
    if !(windows && msvc) {
        return;
    }
    let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        println!("cargo:warning=OUT_DIR not set; udm.exe gets no icon");
        return;
    };
    let path = out_dir.join("udm.res");
    match res_file().map(|bytes| std::fs::write(&path, bytes)) {
        Some(Ok(())) => println!("cargo:rustc-link-arg-bin=udm={}", path.display()),
        _ => println!("cargo:warning=cannot write udm.res; udm.exe gets no icon"),
    }
}
