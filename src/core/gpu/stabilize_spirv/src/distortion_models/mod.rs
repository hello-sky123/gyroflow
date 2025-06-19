// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

pub mod opencv_fisheye;
pub mod opencv_standard;
pub mod poly3;
pub mod poly5;
pub mod ptlens;
pub mod insta360;
pub mod sony;

pub mod gopro_superview;
pub mod gopro_hyperview;
pub mod digital_stretch;

use crate::KernelParams;
use crate::glam::{ Vec2, Vec3 };

// 自动创建一个统一的系统来管理和调用多种不同的镜头畸变模型
macro_rules! impl_models {
    // $()*表示括号内的模式可以重复多次，$id:tt匹配任何token序列，命名为id，::字面双冒号，命名空间分隔符
    ($($id:tt::$name:tt,)*) => {
        #[derive(Clone, Copy)]
        #[repr(i32)] // 告诉编译器枚举在内存中使用i32整数来表示，GPU理解的是数字，而不是Rust的枚举类型
        pub enum DistortionModel {
            $($name,)*
        }

        // 为DistortionModel实现Default trait，指定一个默认的畸变模型。这里硬编码为OpenCVFisheye
        impl Default for DistortionModel {
            fn default() -> Self { Self::OpenCVFisheye }
        }

        // 为DistortionModel枚举实现了多个方法
        impl DistortionModel {
            pub fn undistort_point(&self, point: Vec2, params: &KernelParams) -> Vec2 {
                // 它match当前的枚举值，$()*部分会为宏输入的每一项生成一个match分支
                match &self {
                    // 根据当前的DistortionModel类型调用对应模块下的undistort_point方法
                    $(DistortionModel::$name => <$id::$name>::undistort_point(point, params),)*
                }
            }
            pub fn distort_point(&self, point: Vec3, params: &KernelParams) -> Vec2 {
                match &self {
                    $(DistortionModel::$name => <$id::$name>::distort_point(point, params),)*
                }
            }

            #[cfg(not(target_arch = "spirv"))] // spirv是GPU着色器的一种中间语言，cpu编译时执行以下代码
            pub fn adjust_lens_profile(&self, calib_w: &mut usize, calib_h: &mut usize/*, lens_model: &mut String*/) {
                match &self {
                    $(DistortionModel::$name => <$id::$name>::adjust_lens_profile(calib_w, calib_h/*, lens_model*/),)*
                }
            }
            #[cfg(not(target_arch = "spirv"))]
            pub fn from_name(id: &str) -> Self {
                $(
                    if stringify!($id) == id { return Self::$name; }
                )*
                Self::default()
            }
        }
    };
}

impl_models! {
    none::None,

    // Physical lenses
    opencv_fisheye::OpenCVFisheye,
    opencv_standard::OpenCVStandard,
    poly3::Poly3,
    poly5::Poly5,
    ptlens::PtLens,
    insta360::Insta360,
    sony::Sony,

    // Digital lenses (ie. post-processing)
    gopro_superview::GoProSuperview,
    gopro_hyperview::GoProHyperview,
    digital_stretch::DigitalStretch,
}

mod none {
    use crate::glam::{ Vec2, Vec3 };
    pub struct None { }
    impl None {
        #[inline] pub fn undistort_point(p: Vec2, _: &crate::KernelParams) -> Vec2 { p }
        #[inline] pub fn distort_point(p: Vec3, _: &crate::KernelParams) -> Vec2 { Vec2::new(p.x, p.y) }
        #[cfg(not(target_arch = "spirv"))] pub fn adjust_lens_profile(_: &mut usize, _: &mut usize/*, _: &mut String*/) { }
    }
}
