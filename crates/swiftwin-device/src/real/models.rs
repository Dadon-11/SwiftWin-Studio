//! Screen sizes for iPhone product types (points + scale). Only a fallback:
//! the real size comes from WebDriverAgent's window size / screenshot pixels.
//! Values from Apple's Human Interface Guidelines "Layout > Specifications".

use swiftwin_core::ScreenSpec;

pub fn screen_for(product_type: &str) -> Option<ScreenSpec> {
    let (w, h, s) = match product_type {
        "iPhone12,8" | "iPhone14,6" => (375.0, 667.0, 2.0),             // SE 2nd/3rd gen
        "iPhone12,1" => (414.0, 896.0, 2.0),                            // 11
        "iPhone12,3" | "iPhone14,4" => (375.0, 812.0, 3.0),             // 11 Pro, 13 mini
        "iPhone13,1" => (375.0, 812.0, 3.0),                            // 12 mini
        "iPhone12,5" => (414.0, 896.0, 3.0),                            // 11 Pro Max
        "iPhone13,2" | "iPhone13,3" | "iPhone14,2" | "iPhone14,5" | "iPhone14,7" | "iPhone17,5" => (390.0, 844.0, 3.0),
        "iPhone13,4" | "iPhone14,3" | "iPhone14,8" => (428.0, 926.0, 3.0),
        "iPhone15,2" | "iPhone15,4" | "iPhone16,1" | "iPhone17,3" => (393.0, 852.0, 3.0),
        "iPhone15,3" | "iPhone15,5" | "iPhone16,2" | "iPhone17,4" => (430.0, 932.0, 3.0),
        "iPhone17,1" | "iPhone18,1" | "iPhone18,3" => (402.0, 874.0, 3.0), // 16 Pro, 17 Pro, 17
        "iPhone17,2" | "iPhone18,2" => (440.0, 956.0, 3.0),             // 16 Pro Max, 17 Pro Max
        "iPhone18,4" => (420.0, 912.0, 3.0),                            // Air
        p if p.starts_with("iPhone") => (393.0, 852.0, 3.0),           // unknown/newer: common default
        _ => return None,
    };
    Some(ScreenSpec { width_pts: w, height_pts: h, scale: s })
}
