// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use nalgebra::Rotation3;
use crate::stabilization::ComputeParams;
use super::OpticalFlowPair;

mod almeida;            pub use self::almeida::*;
mod eight_point;        pub use self::eight_point::*;
mod find_essential_mat; pub use self::find_essential_mat::*;
mod find_homography;    pub use self::find_homography::*;

// enum_delegate这个库是这里的“魔法”，它能自动将对枚举的调用委托给其内部包含的某个具体算法实例
#[enum_delegate::register]
pub trait EstimatePoseTrait { // 定义所有估计姿态方法的公共接口
    fn init(&mut self, params: &ComputeParams);
    fn estimate_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<Rotation3<f64>>;
}

#[enum_delegate::implement(EstimatePoseTrait)]
#[derive(Clone)]
// 枚举的每一个变体(variant)都代表了一种不同的、具体的姿态估算算法
pub enum EstimatePoseMethod {
    // 这是一个枚举变体，名为PoseFindEssentialMat，包含一个同名的结构体 PoseFindEssentialMat
    // 这个结构体（在别处定义）包含了实现基于本质矩阵 (Essential Matrix) 的姿态估算算法所需的所有状态和逻辑
    PoseFindEssentialMat(PoseFindEssentialMat),
    PoseAlmeida(PoseAlmeida),
    PoseEightPoint(PoseEightPoint),
    PoseFindHomography(PoseFindHomography),
}

// 通过一个数字ID来创建对应的姿态估算算法实例
impl From<u32> for EstimatePoseMethod {
    fn from(v: u32) -> Self {
        match v {
            0 => Self::PoseFindEssentialMat(Default::default()),
            1 => Self::PoseAlmeida(Default::default()),
            2 => Self::PoseEightPoint(Default::default()),
            3 => Self::PoseFindHomography(Default::default()),
            _ => { log::error!("Unknown pose method {v}", ); Self::PoseAlmeida(Default::default()) }
        }
    }
}
