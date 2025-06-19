// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// 从名为biquad的crate中导入多个模块和类型，用于数字滤波器的实现
// trait可以类比为其他语言中的interface或抽象类，trait是一组方法的集合，表示某个类型能做什么，如果希望一个类型
// 具有某些行为。就可以让它实现某个trait。这样就可以用统一的方式来操作实现了该trait的所有类型
use biquad::{Biquad, Coefficients, Type, DirectForm2Transposed, ToHertz};

use super::gyro_source::{ TimeIMU, TimeQuat };

// pub表示这个结构体是公开的，可以从其他模块或crate访问
pub struct Lowpass {
    // 定义结构体字段，数组类型[T; N]表示一个长度为N的数组，元素类型为T
    filters: [DirectForm2Transposed<f64>; 6]
}

// 实现Lowpass结构体的方法
impl Lowpass {
    // 公开的构造函数方法，按照惯例以new命名，其后为参数列表，->符号后是返回类型，Result是Rust的
    // 标准枚举类型，用于可能失败的操作，Ok(T)表示成功，包含结果值，Err(E)表示失败，包含错误信息
    // Result<T, E>是泛型枚举，Self表示当前结构体类型，biquad::Errors是可能的错误类型
    pub fn new(freq: f64, sample_rate: f64) -> Result<Self, biquad::Errors> {
        // let用于变量绑定（声明变量并绑定值），默认是不可变的，如果需要可变变量，可以使用mut关键字
        // Coefficients是biquad crate中的一个泛型结构体，用来保存滤波器的系数（a0, a1, a2, b1, b2）
        // from_params是一个关联函数，用于根据滤波器类型、采样率和截止频率计算系数，?运算符自动处理错误，如果有错误会提前返回Err
        let coeffs = Coefficients::<f64>::from_params(Type::LowPass, sample_rate.hz(), freq.hz(), biquad::Q_BUTTERWORTH_F64)?;
        // 构造并返回一个Lowpass实例，使用Self关键字表示当前类型
        Ok(Self {
            filters: [
                // 创建了一个包含6个滤波器通道的数组，每个通道用相同的coeffs初始化
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
            ]
        })
    }

    // run方法接受一个索引和数据值，返回滤波后的数据值，&mut可变引用，会更新滤波器的内部状态
    pub fn run(&mut self, i: usize, data: f64) -> f64 {
        self.filters[i].run(data)
    }

    // data: &mut [TimeIMU]动态长度数组，表示一组TimeIMU数据
    pub fn filter_gyro(&mut self, data: &mut [TimeIMU]) {
        for x in data {
            // .as_mut():方法调用，获取Option内部数据的可变引用，if let Some(g)模式匹配
            // 当gyro是Some时，绑定内部数组的可变引用到g，当gyro是None时，跳过此代码块
            if let Some(g) = x.gyro.as_mut() {
                g[0] = self.run(0, g[0]); // 调用run方法，传入索引0和数据g[0]，更新g[0]的值
                g[1] = self.run(1, g[1]);
                g[2] = self.run(2, g[2]);
            }

            if let Some(a) = x.accl.as_mut() {
                a[0] = self.run(3, a[0]);
                a[1] = self.run(4, a[1]);
                a[2] = self.run(5, a[2]);
            }
        }
    }

    // 对一组TimeIMU的陀螺仪和加速度计数据，进行两次低通滤波：一次正向，一次反向
    // 这样可以避免滤波带来的相位滞后问题（phase delay），常用于高精度信号处理
    pub fn filter_gyro_forward_backward(freq: f64, sample_rate: f64, data: &mut [TimeIMU]) -> Result<(), biquad::Errors> {
        // 创建两个滤波器实例：一个用于正向滤波，一个用于反向滤波
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        // for x in data.iter_mut()可变迭代器，遍历data中的每个TimeIMU实例
        for x in data.iter_mut() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = forward.run(0, g[0]);
                g[1] = forward.run(1, g[1]);
                g[2] = forward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = forward.run(3, a[0]);
                a[1] = forward.run(4, a[1]);
                a[2] = forward.run(5, a[2]);
            }
        }
        for x in data.iter_mut().rev() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = backward.run(0, g[0]);
                g[1] = backward.run(1, g[1]);
                g[2] = backward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = backward.run(3, a[0]);
                a[1] = backward.run(4, a[1]);
                a[2] = backward.run(5, a[2]);
            }
        }
        Ok(()) // 如果两个滤波器构造成功，就正常返回，否则返回biquad::Errors类型的错误
    }

    // 对一组TimeQuat的四元数数据进行双向低通滤波
    pub fn filter_quats_forward_backward(freq: f64, sample_rate: f64, data: &mut TimeQuat) -> Result<(), biquad::Errors> {
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        for (_ts, uq) in data.iter_mut() {
            let mut q = uq.quaternion().clone(); // 克隆一份四元数作为可变副本
            q.coords[0] = forward.run(0, q.coords[0]);
            q.coords[1] = forward.run(1, q.coords[1]);
            q.coords[2] = forward.run(2, q.coords[2]);
            q.coords[3] = forward.run(3, q.coords[3]);
            *uq = crate::Quat64::from_quaternion(q); // 将滤波后的四元数重新赋值给TimeQuat中的元素
        }
        for (_ts, uq) in data.iter_mut().rev() {
            let mut q = uq.quaternion().clone();
            q.coords[0] = backward.run(0, q.coords[0]);
            q.coords[1] = backward.run(1, q.coords[1]);
            q.coords[2] = backward.run(2, q.coords[2]);
            q.coords[3] = backward.run(3, q.coords[3]);
            *uq = crate::Quat64::from_quaternion(q);
        }
        Ok(())
    }
}

pub struct Median {
    filters: [median::Filter<f64>; 6]
}

impl Median {
    // 创建一个新的Median实例，size是滤波器的窗口大小（历史缓冲区长度）
    pub fn new(size: usize, _sample_rate: f64) -> Self {
        Self {
            filters: [
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
            ]
        }
    }

    pub fn run(&mut self, i: usize, data: f64) -> f64 {
        self.filters[i].consume(data) // 将数据传入对应的中值滤波器，并返回滤波后的结果
    }

    pub fn filter_gyro(&mut self, data: &mut [TimeIMU]) {
        for x in data {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = self.run(0, g[0]);
                g[1] = self.run(1, g[1]);
                g[2] = self.run(2, g[2]);
            }

            if let Some(a) = x.accl.as_mut() {
                a[0] = self.run(3, a[0]);
                a[1] = self.run(4, a[1]);
                a[2] = self.run(5, a[2]);
            }
        }
    }

    // 对一组TimeIMU的陀螺仪和加速度计数据进行双向中值滤波，第一遍（正序）：消除高频噪声，但可能引入延迟
    // 第二遍（反序）：在正序基础上再次滤波，抵消前面的延迟
    pub fn filter_gyro_forward_backward(size: i32, sample_rate: f64, data: &mut [TimeIMU]) {
        // size as _: 将i32类型的size转换为usize类型，_表示编译器自动推断类型
        let mut forward = Self::new(size as _, sample_rate);
        let mut backward = Self::new(size as _, sample_rate);
        for x in data.iter_mut() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = forward.run(0, g[0]);
                g[1] = forward.run(1, g[1]);
                g[2] = forward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = forward.run(3, a[0]);
                a[1] = forward.run(4, a[1]);
                a[2] = forward.run(5, a[2]);
            }
        }
        for x in data.iter_mut().rev() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = backward.run(0, g[0]);
                g[1] = backward.run(1, g[1]);
                g[2] = backward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = backward.run(3, a[0]);
                a[1] = backward.run(4, a[1]);
                a[2] = backward.run(5, a[2]);
            }
        }
    }
}
