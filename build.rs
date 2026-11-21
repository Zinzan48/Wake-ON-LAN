// 零依賴建置腳本：在 Windows 編譯時自動將多解析度向量圖示嵌入 .exe 資源，
// 讓檔案總管、桌面捷徑與工作列都呈現高質感圖示（無需外部 .ico 或 rc.exe）。

include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR missing"));
    let res_path = out_dir.join("app_icon.res");

    let sizes: [u32; 4] = [16, 32, 48, 64];
    let mut res = Vec::new();

    // Win32 .res 檔案開頭固定 32-byte 空資源檔頭
    res.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, // DataSize = 0
        0x20, 0x00, 0x00, 0x00, // HeaderSize = 32
        0xFF, 0xFF, 0x00, 0x00, // Type = 0
        0xFF, 0xFF, 0x00, 0x00, // Name = 0
        0x00, 0x00, 0x00, 0x00, // DataVersion
        0x00, 0x00, 0x00, 0x00, // MemoryFlags + LanguageId
        0x00, 0x00, 0x00, 0x00, // Version
        0x00, 0x00, 0x00, 0x00, // Characteristics
    ]);

    let mut group_entries = Vec::new();

    for (idx, &size) in sizes.iter().enumerate() {
        let icon_id = (idx + 1) as u16;
        let dib = build_icon_dib(size);
        write_res_entry(&mut res, 3 /* RT_ICON */, icon_id, &dib);
        group_entries.push((size, dib.len() as u32, icon_id));
    }

    // 建構 RT_GROUP_ICON (type = 14, id = 1)
    let mut grp = Vec::new();
    grp.extend_from_slice(&0u16.to_le_bytes()); // wReserved
    grp.extend_from_slice(&1u16.to_le_bytes()); // wResType = 1 (ICON)
    grp.extend_from_slice(&(group_entries.len() as u16).to_le_bytes());

    for (size, bytes_in_res, icon_id) in group_entries {
        let dim = if size >= 256 { 0u8 } else { size as u8 };
        grp.push(dim); // bWidth
        grp.push(dim); // bHeight
        grp.push(0); // bColorCount
        grp.push(0); // bReserved
        grp.extend_from_slice(&1u16.to_le_bytes()); // wPlanes
        grp.extend_from_slice(&32u16.to_le_bytes()); // wBitCount
        grp.extend_from_slice(&bytes_in_res.to_le_bytes());
        grp.extend_from_slice(&icon_id.to_le_bytes());
    }

    write_res_entry(&mut res, 14 /* RT_GROUP_ICON */, 1, &grp);

    std::fs::write(&res_path, res).expect("failed to write app_icon.res");
    println!("cargo:rustc-link-arg={}", res_path.display());
}

fn build_icon_dib(size: u32) -> Vec<u8> {
    let n = size as usize;
    let rgba = generate_app_icon_rgba(size);
    let xor_len = n * n * 4;
    let and_stride = ((n + 31) / 32) * 4;
    let and_len = and_stride * n;

    let mut dib = Vec::with_capacity(40 + xor_len + and_len);

    // BITMAPINFOHEADER (40 bytes)
    dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
    dib.extend_from_slice(&(size as i32).to_le_bytes()); // biWidth
    dib.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // biHeight (XOR + AND)
    dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    dib.extend_from_slice(&(xor_len as u32).to_le_bytes()); // biSizeImage
    dib.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    dib.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // XOR BGRA（由下而上）
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            dib.push(rgba[i + 2]); // B
            dib.push(rgba[i + 1]); // G
            dib.push(rgba[i]); // R
            dib.push(rgba[i + 3]); // A
        }
    }

    // AND 1-bit 遮罩（由下而上）
    let mut and_mask = vec![0u8; and_len];
    for y in 0..n {
        let src_y = n - 1 - y;
        for x in 0..n {
            let a = rgba[(src_y * n + x) * 4 + 3];
            if a == 0 {
                and_mask[y * and_stride + (x / 8)] |= 0x80 >> (x % 8);
            }
        }
    }
    dib.extend_from_slice(&and_mask);
    dib
}

fn write_res_entry(out: &mut Vec<u8>, res_type: u16, res_id: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&res_type.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&res_id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&0x1030u16.to_le_bytes()); // MemoryFlags
    out.extend_from_slice(&0x0409u16.to_le_bytes()); // LanguageId
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);

    // 4-byte alignment padding
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}
