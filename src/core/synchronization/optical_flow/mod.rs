// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::OpticalFlowPair;
use std::sync::Arc;

mod akaze;        pub use self::akaze::*;
mod opencv_dis;   pub use opencv_dis::*;
mod opencv_pyrlk; pub use opencv_pyrlk::*;

#[enum_delegate::register] // 向enum_delegate库注册一个trait
pub trait OpticalFlowTrait {
    fn size(&self) -> (u32, u32);
    fn features(&self) -> &Vec<(f32, f32)>;
    fn optical_flow_to(&self, to: &OpticalFlowMethod) -> OpticalFlowPair;
    fn cleanup(&mut self);
    fn can_cleanup(&self) -> bool;
}

// #[]定义一种属性，它作用于紧随其后的代码项
#[enum_delegate::implement(OpticalFlowTrait)] // 第三方库enum_delegate提供的过程宏属性，为枚举实现委托
#[derive(Clone)] // derive是一个内置属性，(Clone)括号内是希望编译器为我们自动实现的trait
// 带数据关联的枚举类型
pub enum OpticalFlowMethod {
    OFAkaze(OFAkaze), // ()表示这个变体有关联数据（一个同名结构体）
    OFOpenCVPyrLK(OFOpenCVPyrLK),
    OFOpenCVDis(OFOpenCVDis),
}

impl OpticalFlowMethod {
    pub fn detect_features(method: u32, timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        match method {
            0 => Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)),
            1 => Self::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(timestamp_us, img, width, height)),
            2 => Self::OFOpenCVDis(OFOpenCVDis::detect_features(timestamp_us, img, width, height)),
            _ => { log::error!("Unknown OF method {method}", ); Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)) }
        }
    }
}
