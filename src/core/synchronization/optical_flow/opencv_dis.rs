// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]
use super::super::OpticalFlowPair;
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use parking_lot::RwLock;
#[cfg(feature = "use-opencv")]
use opencv::{ core::{ Mat, Size, CV_8UC1, Vec2f }, prelude::{ MatTraitConst, DenseOpticalFlowTrait } };

#[derive(Clone)]
pub struct OFOpenCVDis {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (Vec<(f32, f32)>, Vec<(f32, f32)>)>>>,
    timestamp_us: i64,
    size: (i32, i32),
    used: Arc<AtomicU32>,
}

impl OFOpenCVDis {
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        Self {
            features: Vec::new(),
            timestamp_us,
            size: (width as i32, height as i32),
            matched_points: Default::default(),
            img,
            used: Default::default()
        }
    }
}

impl OpticalFlowTrait for OFOpenCVDis {
    fn size(&self) -> (u32, u32) {
        (self.size.0 as u32, self.size.1 as u32)
    }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, _to: &OpticalFlowMethod) -> OpticalFlowPair {
        #[cfg(feature = "use-opencv")]
        // 检查_to枚举是否是OFOpenCVDis变体
        if let OpticalFlowMethod::OFOpenCVDis(next) = _to {
            let (w, h) = self.size;
            // 缓存检查
            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }
            if self.img.is_empty() || next.img.is_empty() || w <= 0 || h <= 0 { return None; }

            // 创建闭包
            let result = || -> Result<(Vec<(f32, f32)>, Vec<(f32, f32)>), opencv::Error> {
                // 使用unsafe创建Mat对象，避免不必要的内存拷贝
                let a1_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(self.img.width() as i32, self.img.height() as i32), CV_8UC1, self.img.as_raw().as_ptr() as *mut std::ffi::c_void, 0) }?;
                let a2_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(next.img.width() as i32, next.img.height() as i32), CV_8UC1, next.img.as_raw().as_ptr() as *mut std::ffi::c_void, 0) }?;

                // 调用OpenCV函数计算稠密光临
                let mut of = Mat::default();
                // 创建DIS的实例，DISOpticalFlow_PRESET_FAST： 预设的、速度优化的参数
                let mut optflow = opencv::video::DISOpticalFlow::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
                optflow.calc(&a1_img, &a2_img, &mut of)?; // 计算光流，结果保存在与图像同样尺寸的of中，类型为CV_32FC2

                // 结果采样与转换（从稠密到稀疏）
                let mut points_a = Vec::new(); // 存储采样点的原始坐标
                let mut points_b = Vec::new(); // 存储采样点运动后的新坐标
                let step = w as usize / 15; // 15 points
                // 以step为步长遍历整个图像，形成一个网格采样
                for i in (0..a1_img.cols()).step_by(step) {
                    for j in (0..a1_img.rows()).step_by(step) {
                        let pt = of.at_2d::<Vec2f>(j, i)?; // 从光流场of中获取网格点(i, j)的运动向量(dx, dy)
                        points_a.push((i as f32, j as f32));
                        points_b.push((i as f32 + pt[0], j as f32 + pt[1]));
                    }
                }
                Ok((points_a, points_b))
            }();

            self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            match result {
                Ok(res) => {
                    self.matched_points.write().insert(next.timestamp_us, res.clone());
                    return Some(res);
                },
                Err(e) => {
                    log::error!("OpenCV error: {:?}", e);
                }
            }
        }
        None
    }
    fn can_cleanup(&self) -> bool {
        self.used.load(std::sync::atomic::Ordering::SeqCst) == 2
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
    }
}
