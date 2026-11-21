use std::env;
use std::fs;
use std::path::Path;

const ICON_ICO: &str = "images/AudioK_imageApp.ico";
const ICON_PNG: &str = "images/AudioK_imageApp_512.png";
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn main() {
    println!("cargo:rerun-if-changed={ICON_ICO}");
    println!("cargo:rerun-if-changed={ICON_PNG}");

    #[cfg(windows)]
    {
        if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            return;
        }
        let icon = normalize_icon(Path::new(ICON_ICO)).expect("icône ICO invalide");
        let out = Path::new(&env::var("OUT_DIR").unwrap()).join("audiok.ico");
        fs::write(&out, icon).unwrap();
        winresource::WindowsResource::new()
            .set_icon(out.to_str().unwrap())
            .compile()
            .expect("compilation des ressources Windows");
    }
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn normalize_icon(path: &Path) -> Result<Vec<u8>, String> {
    let mut data = fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    if data.len() < 6 || le_u16(&data, 0) != 0 || le_u16(&data, 2) != 1 {
        return Err("en-tête ICO invalide".into());
    }
    let count = le_u16(&data, 4) as usize;
    let header_end = 6 + 16 * count;
    if count == 0 || data.len() < header_end {
        return Err("répertoire ICO invalide".into());
    }

    let mut sizes = Vec::with_capacity(count);
    let mut end = header_end;
    for index in 0..count {
        let entry = 6 + 16 * index;
        let bpp = le_u16(&data, entry + 6);
        let size = le_u32(&data, entry + 8) as usize;
        let offset = le_u32(&data, entry + 12) as usize;
        if bpp != 32 {
            return Err(format!("entrée {index}: {bpp} bpp au lieu de 32"));
        }
        if offset != end || size == 0 || offset + size > data.len() {
            return Err(format!("entrée {index}: plage de données invalide"));
        }

        let payload = &data[offset..offset + size];
        let (width, height) = if payload.starts_with(PNG_SIGNATURE) {
            if size < 24 || &payload[12..16] != b"IHDR" {
                return Err(format!("entrée {index}: PNG tronqué ou invalide"));
            }
            (be_u32(payload, 16), be_u32(payload, 20))
        } else {
            if size < 40 || le_u32(payload, 0) < 40 {
                return Err(format!("entrée {index}: bitmap invalide"));
            }
            let height = le_u32(payload, 8);
            if !height.is_multiple_of(2) {
                return Err(format!("entrée {index}: hauteur bitmap invalide"));
            }
            (le_u32(payload, 4), height / 2)
        };
        if width == 0 || width != height || !SIZES.contains(&width) {
            return Err(format!(
                "entrée {index}: dimension {width}×{height} inattendue"
            ));
        }

        let declared = (
            if data[entry] == 0 {
                256
            } else {
                u32::from(data[entry])
            },
            if data[entry + 1] == 0 {
                256
            } else {
                u32::from(data[entry + 1])
            },
        );
        if declared != (width, height) {
            let byte = |value: u32| if value == 256 { 0 } else { value as u8 };
            data[entry] = byte(width);
            data[entry + 1] = byte(height);
            println!(
                "cargo:warning=ICO corrigé: entrée {index} déclarée {}×{} mais contenu {width}×{height}",
                declared.0, declared.1
            );
        }

        sizes.push(width);
        end = offset + size;
    }

    if end != data.len() {
        return Err(format!(
            "{} octets non référencés en fin de fichier",
            data.len() - end
        ));
    }
    sizes.sort_unstable();
    sizes.dedup();
    if sizes.as_slice() != SIZES {
        return Err(format!("tailles {sizes:?}, attendu {SIZES:?}"));
    }
    Ok(data)
}
