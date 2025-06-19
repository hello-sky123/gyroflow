// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use serde::{ Serialize, Deserialize };

// 实时解析嵌入视频文件或来自其他遥测数据的元数据的工具，支持多种格式GoPro, Sony, Insta360等
use telemetry_parser::Input;
use telemetry_parser::tags_impl::{ GetWithType, GroupId, TagId };
use std::io::Result;

// 创建一个精确、唯一的标识，用来描述拍摄某个视频时所使用的相机、镜头和所有相关设置，Gyroflow需要这个精确的指纹来查找并
// 应用完全匹配的镜头配置文件，因为即使是同一台相机，在不同的设置下（如不同的分辨率、帧率或数码变焦模式），其镜头畸变特性也可能完全不同
#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)] // 允许在反序列化时使用默认值填充缺失的字段
pub struct CameraIdentifier {
    pub brand: String, // 相机品牌，如GoPro, Sony, Insta360等
    pub model: String, // 相机型号，可能是Hero 11 Black等
    pub lens_model: String, // 镜头型号，可能是FE 24-70mm F2.8 GM等
    pub lens_info: String, // 镜头信息，可能包含用户备注或从元数据中提取的更详细的描述
    pub focal_length: Option<f64>, // 焦距，单位为毫米，可能是None表示未知或未提供
    // 相机设置，通常指相机的“数码镜头”或“视场角（FOV）”设置。GoPro上，这可能是"SuperView", "Wide", "Linear", "Linear + Horizon Leveling"
    pub camera_setting: String,
    pub fps: usize, // 帧率，单位为帧每秒（FPS），通常是30, 60, 120等
    pub video_width: usize, // 视频分辨率
    pub video_height: usize,
    pub additional: String,

    pub identifier: String // 生成的唯一标识符，通常是一个字符串，包含品牌、型号、镜头信息、分辨率和帧率等信息
}

impl CameraIdentifier {
    // input包含了从视频文件中提取的所有元数据和传感器数据
    pub fn from_telemetry_parser(input: &Input, video_width: usize, video_height: usize, fps: f64) -> Result<Self> {
        let fps = (fps * 1000.0).round() as usize;
        let brand = input.camera_type();
        let model = input.camera_model().cloned().unwrap_or_default();

        let mut id = Self {
            brand: brand.clone(),
            model,
            video_width,
            video_height,
            fps,

            ..Default::default()
        };

        // 将品牌名转换为小写后匹配，如果是runcam或caddx，则将镜头信息设置为"wide"
        match id.brand.to_ascii_lowercase().as_str() {
            "runcam" | "caddx" => id.lens_info = "wide".into(), // into自动推到类型
            _ => { } // _通配符，{}表示不做任何操作
        }

        if !id.brand.is_empty() {
            id.model = id.model.to_string().replace(&id.brand, "").trim().to_string();
        }

        match brand.as_str() {
            // 解析和翻译GoPro视频文件中独特的元数据，并将这些信息填充到CameraIdentifier中
            "GoPro" => {
                if let Some(ref samples) = input.samples {
                    for info in samples {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Unknown(0x45495341/*EISA*/)) as Option<&String> {
                                    if v != "N/A" {
                                        id.additional = if v == "Y" || v == "N" {
                                            format!("EIS-{}", v)
                                        } else {
                                            v.clone()
                                        };
                                    }
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x45495345/*EISE*/)) as Option<&String> {
                                    if id.additional.is_empty() {
                                        id.additional = format!("EIS-{}", v);
                                    }
                                }
                                if id.additional == "EIS-N" {
                                    id.additional = "NO-EIS".into();
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x56464f56/*VFOV*/)) as Option<&String> {
                                    match v.as_str() {
                                        "X" => id.lens_info = "Max".into(),
                                        "W" => id.lens_info = "Wide".into(),
                                        "S" => id.lens_info = "Super".into(),
                                        "H" => id.lens_info = "Hyper".into(),
                                        "L" => id.lens_info = "Linear".into(),
                                        "N" => id.lens_info = "Narrow".into(),
                                        "M" => id.lens_info = "Medium".into(),
                                        _ => id.lens_info = v.into()
                                    };
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x5a464f56/*ZFOV*/)) as Option<&f32> {
                                    if id.lens_info == "Linear" && *v < 80.0 {
                                        id.lens_info = "Narrow".into();
                                    }
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x50524a54/*PRJT*/)) as Option<&String> {
                                    if v.as_str() == "GPMW" {
                                        id.lens_info = "Max Wide".into();
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            },
            "Sony" => {
                if let Some(ref samples) = input.samples {
                    if let Some(info) = samples.iter().next() {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::FocalLength) as Option<&f32>) {
                                id.lens_info = format!("{:.2} mm", v);
                                id.focal_length = Some(*v as f64);
                            }
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::DisplayName) as Option<&String>) {
                                id.lens_model = v.clone();
                            }
                            if let Some(v) = tag_map.get(&GroupId::Custom("LensDistortion".into())).and_then(|map| map.get_t(TagId::Data) as Option<&serde_json::Value>) {
                                if id.lens_info.is_empty() {
                                    let mut hasher = crc32fast::Hasher::new();
                                    // use previous json for hash to load previous lens profiles
                                    if let Some(v) = v.as_object() {
                                        if v.contains_key("focal_length_nm") {
                                            let s = serde_json::json!({
                                                "unk1": [v["focal_length_nm"], v["effective_sensor_height_nm"]],
                                                "unk2": v["unk1"],
                                                "unk3": v["coeff_scale"],
                                                "unk4": v["coeffs"]
                                            }).to_string();
                                            hasher.update(s.as_bytes());

                                            id.lens_info = format!("{:x}", hasher.finalize());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "Insta360" => {
                if let Some(ref samples) = input.samples {
                    for info in samples {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Metadata) as Option<&serde_json::Value> {
                                    if let Some(fov_type) = v.get("fov_type").and_then(|v| v.as_str()) {
                                        id.lens_info = fov_type.replace("FovType", "");
                                    }
                                    if let Some(fov) = v.get("fov").and_then(|v| v.as_f64()) {
                                        if fov > 0.0 {
                                            id.lens_info.push_str(&format!(" {:.0}", fov));
                                        }
                                    }
                                    if let Some(flowstate) = v.get("is_flowstate_online").and_then(|v| v.as_bool()) {
                                        id.additional = if flowstate { "EIS" } else { "NO-EIS" }.into();
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(ref samples) = input.samples {
                    let mut try_again = false;
                    for info in samples {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::FocalLength) as Option<&f32>) {
                                id.lens_info = format!("{:.2} mm", v);
                                id.focal_length = Some(*v as f64);
                            }
                            if brand != "Runcam" {
                                if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::Name) as Option<&String>) {
                                    id.lens_model = v.clone();
                                }
                            }
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Metadata) as Option<&serde_json::Value> {
                                    log::debug!("Camera ID Brand: {}, Model: {}, Metadata: {:?}", id.brand, id.model, v);
                                    if let Some(v) = v.get("lens_info")             .and_then(|v| v.as_str()) { id.lens_info      = v.to_string(); }
                                    if let Some(v) = v.get("focal_length")          .and_then(|v| v.as_f64()) { id.lens_info      = format!("{:.2}mm", v); id.focal_length = Some(v); }
                                    if let Some(v) = v.get("focal_length")          .and_then(|v| v.as_str()) { id.lens_info      = v.to_string(); id.focal_length = v.replace("mm", "").parse::<f64>().ok(); }
                                    if let Some(v) = v.get("lens_type")             .and_then(|v| v.as_str()) { id.lens_model     = v.to_string(); }
                                    if let Some(v) = v.get("resolution_format_name").and_then(|v| v.as_str()) { id.camera_setting = v.to_string(); }
                                }
                            }
                        }
                        if id.lens_info.is_empty() && !try_again {
                            try_again = true;
                            continue;
                        }
                        break;
                    }
                }
            }
        }

        id.identifier = id.get_identifier();

        log::debug!("{:#?}", id);

        Ok(id)
    }

    pub fn get_identifier_for_autoload(&self) -> String {
        self.identifier.replace("hero12", "hero11")
                       .replace("hero13", "hero11")
                       .replace("hero11blackmini", "hero11black")
    }

    fn get_identifier(&self) -> String {
        if self.brand.is_empty() || self.model.is_empty() || self.lens_info.is_empty() { return String::new(); }
        let fps = match self.brand.as_ref() {
            "RED" | "RED RAW" => 0, // RED doesn't do any sensor crop while maintaining the resolution, so we can skip fps
            _ => self.fps
        };

        let mut id = format!("{}-{}-{}-{}-{}x{}@{}-{}", self.brand, self.model, self.lens_model, self.lens_info, self.video_width, self.video_height, fps, self.additional);
        id = id.replace(' ', "");
        id = id.replace("--", "-");
        id = id.replace("--", "-");
        let x: &[_] = &['-', ' '];
        id.trim_matches(x).to_lowercase()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
