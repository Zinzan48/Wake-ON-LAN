/// 高質感向量反鋸齒 Wake-on-LAN 圖示產生器（純 Rust、零依賴，供執行期與 build.rs 共用）
/// 使用 4×4 超採樣反鋸齒（SSAA）與 SDF 距離場繪製圓角漸層底座與發光電源／訊號符號。

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp01((x - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_rgb(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

fn blend_over(dst: [f32; 4], src_rgb: [f32; 3], src_a: f32) -> [f32; 4] {
    let sa = clamp01(src_a);
    let out_a = sa + dst[3] * (1.0 - sa);
    if out_a <= 1e-6 {
        return [0.0; 4];
    }
    let r = (src_rgb[0] * sa + dst[0] * dst[3] * (1.0 - sa)) / out_a;
    let g = (src_rgb[1] * sa + dst[1] * dst[3] * (1.0 - sa)) / out_a;
    let b = (src_rgb[2] * sa + dst[2] * dst[3] * (1.0 - sa)) / out_a;
    [r, g, b, out_a]
}

/// 圓角矩形 SDF（半寬 b，圓角半徑 r）
fn sd_rounded_box(px: f32, py: f32, bx: f32, by: f32, r: f32) -> f32 {
    let qx = px.abs() - bx + r;
    let qy = py.abs() - by + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

/// 線段（膠囊）SDF
fn sd_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let pax = px - ax;
    let pay = py - ay;
    let bax = bx - ax;
    let bay = by - ay;
    let denom = bax * bax + bay * bay;
    let h = if denom > 1e-6 {
        clamp01((pax * bax + pay * bay) / denom)
    } else {
        0.0
    };
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    (dx * dx + dy * dy).sqrt()
}

/// 開口圓弧 SDF（上方開口，附圓潤端點）
/// half_gap_rad 為上方中心向左右張開的半角（弧度）
fn sd_open_ring(px: f32, py: f32, radius: f32, half_gap_rad: f32) -> f32 {
    // 旋轉座標系使上方 (-Y) 對應角度 0
    let ax = px.abs();
    let ay = -py;
    let angle = ax.atan2(ay); // 0 = 正上方, PI = 正下方
    if angle >= half_gap_rad {
        (px * px + py * py).sqrt().abs() - radius
    } else {
        // 距離圓弧端點的歐氏距離（自然形成圓角端帽）
        let ex = radius * half_gap_rad.sin();
        let ey = -radius * half_gap_rad.cos();
        let dx = ax - ex;
        let dy = py - ey;
        (dx * dx + dy * dy).sqrt()
    }
}

/// 產生指定尺寸（width = height = size）的高質感 RGBA8 圖示像素
pub fn generate_app_icon_rgba(size: u32) -> Vec<u8> {
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    let inv_size = 1.0 / (size as f32);
    let aa = 1.35 * inv_size;

    const SUB: usize = 4;
    let sub_step = 1.0 / (SUB as f32);

    for y in 0..n {
        for x in 0..n {
            let mut acc = [0.0f32; 4];

            for sy in 0..SUB {
                for sx in 0..SUB {
                    // 正規化座標至 [-1.0, 1.0]
                    let u = ((x as f32) + ((sx as f32) + 0.5) * sub_step) * inv_size * 2.0 - 1.0;
                    let v = ((y as f32) + ((sy as f32) + 0.5) * sub_step) * inv_size * 2.0 - 1.0;

                    let mut col = [0.0f32; 4];

                    // 1. 柔和圓角方形背景（深海藍 -> 電光藍 -> 亮青漸層）
                    let d_box = sd_rounded_box(u, v, 0.90, 0.90, 0.27);
                    let box_alpha = 1.0 - smoothstep(-aa, aa, d_box);
                    if box_alpha > 0.0 {
                        let grad_t = clamp01((u * 0.55 - v * 0.65 + 0.95) * 0.52);
                        let c_dark = [0.06, 0.11, 0.25];
                        let c_mid = [0.11, 0.34, 0.78];
                        let c_cyan = [0.04, 0.69, 0.85];
                        let mut bg = if grad_t < 0.55 {
                            lerp_rgb(c_dark, c_mid, grad_t / 0.55)
                        } else {
                            lerp_rgb(c_mid, c_cyan, (grad_t - 0.55) / 0.45)
                        };

                        // 左上柔和玻璃高光
                        let spec_dist = ((u + 0.32).powi(2) + (v + 0.45).powi(2)).sqrt();
                        let spec = (1.0 - smoothstep(0.0, 1.05, spec_dist)) * 0.20;
                        bg = [
                            clamp01(bg[0] + spec),
                            clamp01(bg[1] + spec * 1.05),
                            clamp01(bg[2] + spec * 1.15),
                        ];

                        // 精緻內描邊（增加立體質感）
                        let rim_dist = (d_box + 0.032).abs();
                        let rim = (1.0 - smoothstep(0.0, 0.038 + aa, rim_dist)) * 0.26;
                        bg = lerp_rgb(bg, [0.75, 0.95, 1.0], rim);

                        col = blend_over(col, bg, box_alpha);
                    }

                    // 2. Wake-on-LAN 電源符號（圓弧 + 頂部直線，微向下偏移視覺置中）
                    let cy = v - 0.04;
                    let d_ring = sd_open_ring(u, cy, 0.44, 0.56).abs() - 0.088;
                    let d_stem = sd_segment(u, cy, 0.0, -0.58, 0.0, -0.10) - 0.088;
                    let d_power = d_ring.min(d_stem);

                    // 青色霓虹外發光（僅在圖示內部）
                    if box_alpha > 0.0 {
                        let glow = (1.0 - smoothstep(0.0, 0.24, d_power.max(0.0))) * 0.42 * box_alpha;
                        col = blend_over(col, [0.35, 0.90, 1.0], glow);
                    }

                    // 銳利純白電源主體（帶微淡青色邊緣過渡）
                    let power_alpha = 1.0 - smoothstep(-aa, aa, d_power);
                    if power_alpha > 0.0 {
                        let white_tint = lerp_rgb([0.88, 0.98, 1.0], [1.0, 1.0, 1.0], clamp01(-v * 0.4 + 0.65));
                        col = blend_over(col, white_tint, power_alpha);
                    }

                    // 3. 底部細微喚醒脈衝圓弧（象徵網路喚醒訊號）
                    if box_alpha > 0.0 {
                        let pulse_dist = ((u * u + (cy - 0.02).powi(2)).sqrt() - 0.68).abs() - 0.024;
                        let angle_bottom = u.abs().atan2(cy - 0.02); // 0 = 正下方
                        if angle_bottom < 0.52 {
                            let edge_fade = 1.0 - smoothstep(0.32, 0.52, angle_bottom);
                            let pulse_a = (1.0 - smoothstep(-aa, aa, pulse_dist)) * 0.55 * edge_fade * box_alpha;
                            col = blend_over(col, [0.55, 0.95, 1.0], pulse_a);
                        }
                    }

                    // 以 premultiplied alpha 累加超採樣
                    acc[0] += col[0] * col[3];
                    acc[1] += col[1] * col[3];
                    acc[2] += col[2] * col[3];
                    acc[3] += col[3];
                }
            }

            let samples = (SUB * SUB) as f32;
            let a = acc[3] / samples;
            let idx = (y * n + x) * 4;
            if a > 1e-5 {
                let r = clamp01((acc[0] / samples) / a);
                let g = clamp01((acc[1] / samples) / a);
                let b = clamp01((acc[2] / samples) / a);
                out[idx] = (r * 255.0 + 0.5) as u8;
                out[idx + 1] = (g * 255.0 + 0.5) as u8;
                out[idx + 2] = (b * 255.0 + 0.5) as u8;
                out[idx + 3] = (clamp01(a) * 255.0 + 0.5) as u8;
            }
        }
    }

    out
}
