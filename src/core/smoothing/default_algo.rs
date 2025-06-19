// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Aphobius

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Multiply max velocity (500 deg/s) with slider value
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is interpolated between 1s smoothness at 0 velocity, 0.1s smoothness at max velocity and extrapolated above that
// 5. This way, low velocities are smoothed using 1s smoothness, but high velocities are smoothed using 0.1s smoothness at max velocity (500 deg/s multiplied by slider) and gradually lower smoothness above that
// 6. Calculate distance from smoothed quaternions to raw quaternions
// 7. Normalize distance and set everything bellow 0.5 to 0.0
// 8. Smooth distance
// 9. Normalize distance again and change range to 0.5 - 1.0
// 10. Perform plain 3D smoothing, on the last smoothed quaternions, with varying alpha, interpolated between 1s and 0.1s smoothness based on previously calculated velocity multiplied by the distance

use std::collections::BTreeMap;

use super::*;
use nalgebra::*;
use crate::keyframes::*;

const MAX_VELOCITY: f64 = 500.0;
// Use 120 diagonal FOV as reference. Anything below (long focal length) scales the smoothness down. Anything above (short focal length) scales the smoothness up.
// This is needed, because the same rotation at long focal length will be much larger actual image rotation than at short focal length.
const FOV_REFERENCE: f64 = 120.0;
const RAD_TO_DEG: f64 = 180.0 / std::f64::consts::PI;

// 自动为结构体DefaultAlgo实现Clone trait（特性），可以方便的复制结构体的实例
#[derive(Clone)]
pub struct DefaultAlgo {
    // 最主要的平滑度参数，控制着视频防抖的整体强度，数值越高，画面就会越平滑，同时也会导致更大程度的裁剪（画面缩放）
    pub smoothness: f64, // per_axis为false时，这个参数会作用于所有轴
    pub smoothness_pitch: f64, // 俯仰轴的平滑度
    pub smoothness_yaw: f64, // 偏航轴的平滑度
    pub smoothness_roll: f64, // 翻滚轴的平滑度
    pub per_axis: bool, // 是否为俯仰、偏航和翻滚轴分别设置平滑度
    pub second_pass: bool, // 是否进行第二次平滑处理
    pub trim_range_only: bool, // 是否仅在设置裁剪时间范围内进行平滑处理
    pub max_smoothness: f64, // 高速运动时的最大平滑度，它的作用是限制相机在高速运动时所施加的最大平滑度，避免过度裁剪
    pub alpha_0_1s: f64, // 低通滤波器的alpha值
}

impl Default for DefaultAlgo {
    fn default() -> Self { Self {
        smoothness: 0.5,
        smoothness_pitch: 0.5,
        smoothness_yaw: 0.5,
        smoothness_roll: 0.5,
        per_axis: false,
        second_pass: true,
        trim_range_only: true,
        max_smoothness: 1.0,
        alpha_0_1s: 0.1
    } }
}

impl SmoothingAlgorithm for DefaultAlgo {
    // 返回算法的名称，to_owned()将不可变的静态的字符串转换为拥有所有权的String类型
    fn get_name(&self) -> String { "Default".to_owned() }

    // 提供一个统一的接口来动态的修改结构体DefaultAlgo实例中的某一个参数，通过字符串name来指定要修改的参数名称，val是新的值
    fn set_parameter(&mut self, name: &str, val: f64) {
        // Rust的控制流结构，类似于switch-case语句，根据参数名称name来匹配不同的情况，并将val赋值给对应的字段
        match name {
            "smoothness"       => self.smoothness = val,
            "smoothness_pitch" => self.smoothness_pitch = val,
            "smoothness_yaw"   => self.smoothness_yaw = val,
            "smoothness_roll"  => self.smoothness_roll = val,
            "per_axis"         => self.per_axis = val > 0.1,
            // "second_pass"      => self.second_pass = val > 0.1,
            "trim_range_only"  => self.trim_range_only = val > 0.1,
            "max_smoothness"   => self.max_smoothness = val,
            "alpha_0_1s"       => self.alpha_0_1s = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }

    // 根据参数名称name返回对应的值，如果参数不存在，则返回0.0
    fn get_parameter(&self, name: &str) -> f64 {
        match name {
            "smoothness"       => self.smoothness,
            "smoothness_pitch" => self.smoothness_pitch,
            "smoothness_yaw"   => self.smoothness_yaw,
            "smoothness_roll"  => self.smoothness_roll,
            "per_axis"         => if self.per_axis { 1.0 } else { 0.0 },
            // "second_pass"      => if self.second_pass { 1.0 } else { 0.0 },
            "trim_range_only"  => if self.trim_range_only { 1.0 } else { 0.0 },
            "max_smoothness"   => self.max_smoothness,
            "alpha_0_1s"       => self.alpha_0_1s,
            // _是一个通配符，它会匹配任何之前没有被匹配到的值
            _ => 0.0
        }
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        // serde_json::json!是一个宏，它提供了一种非常便利的方式，可以直接在Rust代码中编写JSON。
        // 最外层的[...]表示这个宏将创建一个JSON数组。数组中的每一个{...}都是一个JSON对象，代表着一个可配置的参数
        serde_json::json!([
            // 滑块类型（smoothness）
            {
                "name": "smoothness", // 参数的内部名称，用于程序识别，与set_parameter方法中的名称对应
                "description": "Smoothness", // 参数的显示名称，会显示在UI上给用户看
                "type": "SliderWithField", // 指定UI应该生成一个带输入框的滑块组件
                "from": 0.001, // 定义滑块的最小和最大值范围
                "to": 1.0,
                "value": self.smoothness, // 当前值，来自结构体的字段
                "default": 0.5, // 默认值，如果用户没有修改过这个参数，则使用这个值
                "unit": "", // 值的单位，这里没有单位
                "precision": 3, // 数值显示时的小数点精度
                "keyframe": "SmoothingParamSmoothness" // 这个参数的关键帧类型，用于视频编辑软件中的关键帧动画
            },
            {
                "name": "smoothness_pitch",
                "description": "Pitch smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_pitch,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamPitch"
            },
            {
                "name": "smoothness_yaw",
                "description": "Yaw smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_yaw,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamYaw"
            },
            {
                "name": "smoothness_roll",
                "description": "Roll smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_roll,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamRoll"
            },
            // 复选框类型（per_axis）
            {
                "name": "per_axis",
                "description": "Per axis",
                "advanced": true, // 表示这应该是高级选项，在UI中可能默认隐藏或折叠
                "type": "CheckBox", // 指定UI应该生成一个复选框组件
                "default": self.per_axis,
                "value": if self.per_axis { 1.0 } else { 0.0 },
                // QML是一种用于构建用户界面的标记语言，主要用于Qt框架。Gyroflow的界面就是用Qt/QML构建的。
                // 这段QML代码定义了当这个复选框状态改变时触发的自定义逻辑。它的作用是：
                // 当"Per axis"复选框被勾选时，隐藏主"smoothness"滑块，同时显示俯仰、偏航、翻滚三个轴各自的平滑度滑块。
                // 当它被取消勾选时，则反过来操作。
                // 这完美地展示了这个JSON如何驱动一个动态的、有交互逻辑的UI
                "custom_qml": "Connections { function onCheckedChanged() {
                    const checked = root.getParamElement('per_axis').checked;
                    root.getParamElement('smoothness-label').visible = !checked;
                    root.getParamElement('smoothness_pitch-label').visible = checked;
                    root.getParamElement('smoothness_yaw-label').visible = checked;
                    root.getParamElement('smoothness_roll-label').visible = checked;
                }}"
            },
            /*{
                "name": "second_pass",
                "description": "Second smoothing pass",
                "advanced": true,
                "type": "CheckBox",
                "default": self.second_pass,
                "value": if self.second_pass { 1.0 } else { 0.0 },
            },*/
            {
                "name": "trim_range_only",
                "description": "Only within trim range",
                "advanced": true,
                "type": "CheckBox",
                "default": self.trim_range_only,
                "value": if self.trim_range_only { 1.0 } else { 0.0 },
            },
            {
                "name": "max_smoothness",
                "description": "Max smoothness",
                "advanced": true,
                "type": "SliderWithField",
                "from": 0.1,
                "to": 5.0,
                "value": self.max_smoothness,
                "default": 1.0,
                "precision": 3,
                "unit": "s",
                "keyframe": "SmoothingParamTimeConstant"
            },
            {
                "name": "alpha_0_1s",
                "description": "Max smoothness at high velocity",
                "advanced": true,
                "type": "SliderWithField",
                "from": 0.01,
                "to": 1.0,
                "value": self.alpha_0_1s,
                "default": 0.1,
                "precision": 3,
                "unit": "s",
                "keyframe": "SmoothingParamTimeConstant2"
            }
        ])
    }

    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    // 为当前DefaultAlgo结构体中所有重要的参数设置，计算一个唯一的、紧凑的数字标识符，可以用于性能优化和缓存
    // 如果两个DefaultAlgo实例的get_checksum()返回值相同，则可以认为它们的参数设置是相同的
    fn get_checksum(&self) -> u64 {
        // 创建一个默认的哈希计算器
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // 将每个参数的值转换为64位整数，并写入哈希计算器
        hasher.write_u64(self.smoothness.to_bits());
        hasher.write_u64(self.smoothness_pitch.to_bits());
        hasher.write_u64(self.smoothness_yaw.to_bits());
        hasher.write_u64(self.smoothness_roll.to_bits());
        hasher.write_u64(self.max_smoothness.to_bits());
        hasher.write_u64(self.alpha_0_1s.to_bits());
        hasher.write_u8(if self.per_axis { 1 } else { 0 });
        hasher.write_u8(if self.second_pass { 1 } else { 0 });
        hasher.finish() // 返回计算得到的哈希值
    }

    // quats是一个时间戳和四元数的映射，duration_ms是平滑处理的持续时间，compute_params包含计算所需的其他参数
    fn smooth(&self, quats: &TimeQuat, duration_ms: f64, compute_params: &ComputeParams) -> TimeQuat { // TODO Result<>?
        // 如果quats为空或者duration_ms小于等于0，则直接返回原始的quats
        if quats.is_empty() || duration_ms <= 0.0 { return quats.clone(); }

        let sample_rate: f64 = quats.len() as f64 / (duration_ms / 1000.0); // 计算采样率
        let rad_to_deg_per_sec: f64 = sample_rate * RAD_TO_DEG;

        // 定义一个闭包（lambda函数），用于根据时间常数计算低通滤波器的alpha值
        // 这是指数移动平均（EMA）滤波器的标准公式，alpha值决定了滤波器的平滑程度
        let get_alpha = |time_constant: f64| {
            1.0 - (-(1.0 / sample_rate) / time_constant).exp()
        };
        let noop = |v| v; // 一个空操作闭包，直接返回输入值

        // 获取对关键帧数据的引用，关键帧允许用户在视频的不同时间点设置不同的平滑参数
        let keyframes = &compute_params.keyframes;

        // 根据用户设定的修剪范围，对输入的四元数数据进行裁剪
        let quats = Smoothing::get_trimmed_quats(quats, compute_params.scaled_duration_ms, self.trim_range_only, &compute_params.trim_ranges);
        let quats = quats.as_ref(); // 将裁剪后的数据（可能是一个Cow<T>类型）转换为引用，方便后续使用

        // 定义一个非常重要的闭包，用于获取随时间动态变化（由关键帧控制）的参数值。typ: 参数类型（如平滑度、时间常数等）
        // def: 如果没有关键帧，使用的默认值，cb: 一个回调函数（比如上面的 get_alpha 或 noop），用于对最终值进行处理。
        // 返回值是一个BTreeMap，键是时间戳，值是该时间戳对应的参数值。
        let get_keyframed_param = |typ: &KeyframeType, def: f64, cb: &dyn Fn(f64) -> f64| -> BTreeMap<i64, f64> {
            let mut ret = BTreeMap::<i64, f64>::new();
            // 检查这个参数是否有关键帧，或者视频速度是否影响平滑度且视频速度不为1.0
            if keyframes.is_keyframed(typ) || (compute_params.video_speed_affects_smoothing && (compute_params.video_speed != 1.0 || keyframes.is_keyframed(&KeyframeType::VideoSpeed))) {
                // iter创建一个迭代器，迭代器是一个可以逐个产生值的序列，在这里它逐个产生(&i64, &Quat64)元组借用
                // 在Rust中，core::iter::traits::iterator::Iterator是所有迭代器的基础trait，其中最常用的方法之一就是map
                // map方法将一个函数（闭包）应用到迭代器的每个元素上，并返回一个新的迭代器
                ret = quats.iter().map(|(ts, _)| {
                    let timestamp_ms = *ts as f64 / 1000.0;
                    // 获取当前时间戳下的参数值，如果没有关键帧则使用默认值
                    let mut val = keyframes.value_at_gyro_timestamp(typ, timestamp_ms).unwrap_or(def);
                    // 如果视频速度影响平滑
                    if compute_params.video_speed_affects_smoothing {
                        // 获取当前时间的视频播放速度
                        let vid_speed = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoSpeed, timestamp_ms).unwrap_or(compute_params.video_speed).abs();
                        // 根据速度调整时间常数参数，以在不同速度下保持相似的平滑“感觉”
                        if typ == &KeyframeType::SmoothingParamTimeConstant || typ == &KeyframeType::SmoothingParamTimeConstant2 {
                            val *= 1.0 + ((vid_speed - 1.0) / 2.0);
                        } else {
                            val *= vid_speed; // 其他参数直接按速度比例缩放
                        }
                    }
                    // 将处理后的值通过回调函数（cb）并与时间戳一起存入BTreeMap
                    (*ts, cb(val)) // map闭包的返回值，是一个元组，.collect()是一个消费器，它会消耗掉整个迭代器，并将所有元素收集到一个集合中
                }).collect();
            }
            ret // 在Rust中，函数或闭包的最后一个表达式（没有分号结尾）会自动成为其返回值
        };

        // 使用上面的闭包，为每个可变参数生成时间序列映射
        // 主要平滑因子的时间序列
        let alpha_smoothness_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamTimeConstant, self.max_smoothness, &get_alpha);
        // 次要平滑因子（用于速度平滑）的时间序列
        let alpha_0_1s_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamTimeConstant2, self.alpha_0_1s, &get_alpha);
        // 各轴向平滑度参数的时间序列
        let smoothness_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamSmoothness, self.smoothness, &noop);
        let smoothness_pitch_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamPitch, self.smoothness_pitch, &noop);
        let smoothness_yaw_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamYaw, self.smoothness_yaw, &noop);
        let smoothness_roll_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamRoll, self.smoothness_roll, &noop);

        // 计算默认的alpha值，当没有关键帧时使用
        let alpha_smoothness = get_alpha(self.max_smoothness);
        let alpha_0_1s = get_alpha(self.alpha_0_1s);

        // Calculate velocity
        let mut velocity = BTreeMap::<i64, Vector3<f64>>::new();

        // 第一个样本的速度为0，next()获取迭代器中的第一个元素
        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, Vector3::from_element(0.0));

        // 遍历四元数，计算每个样本相对于前一个样本的旋转变化，作为角速度
        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) { // 返回一个迭代器，跳过第一个元素，从第二个元素开始
            let dist = prev_quat.inverse() * quat; // 计算两个姿态之间的差异四元数
            if self.per_axis { // 如果是分轴平滑
                let euler = dist.euler_angles(); // 将差异转换为欧拉角（俯仰、偏航、翻滚）
                velocity.insert(*timestamp, Vector3::new(
                    euler.0.abs() * rad_to_deg_per_sec, // 分别计算各轴的角速度（度/秒）
                    euler.1.abs() * rad_to_deg_per_sec,
                    euler.2.abs() * rad_to_deg_per_sec
                ));
            } else { // 如果是统一平滑
                // 直接使用总的旋转角度作为速度
                velocity.insert(*timestamp, Vector3::from_element(dist.angle() * rad_to_deg_per_sec));
            }
            prev_quat = *quat;
        }

        // Smooth velocity
        // 使用双向低通滤波器对计算出的速度进行平滑，以消除速度数据中的噪声
        // 双向滤波可以防止产生相位延迟（即平滑后的数据不会滞后于原始数据）
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        // 正向传递
        for (_timestamp, vel) in velocity.iter_mut().skip(1) {
            *vel = prev_velocity * (1.0 - alpha_0_1s) + *vel * alpha_0_1s;
            prev_velocity = *vel;
        }
        // 反向传递
        for (_timestamp, vel) in velocity.iter_mut().rev().skip(1) {
            *vel = prev_velocity * (1.0 - alpha_0_1s) + *vel * alpha_0_1s;
            prev_velocity = *vel;
        }

        // Normalize velocity
        // 这是自适应平滑的关键。它将速度转换为一个0到1之间的比例值
        // 该比例值表示当前运动是属于无意的“抖动”还是有意的“运镜”
        for (ts, vel) in velocity.iter_mut() {
            // 获取当前时间点的各轴平滑度设置
            let smoothness_pitch = smoothness_pitch_per_timestamp.get(ts).unwrap_or(&self.smoothness_pitch);
            let smoothness_yaw   = smoothness_yaw_per_timestamp  .get(ts).unwrap_or(&self.smoothness_yaw);
            let smoothness_roll  = smoothness_roll_per_timestamp .get(ts).unwrap_or(&self.smoothness_roll);
            let smoothness       = smoothness_per_timestamp      .get(ts).unwrap_or(&self.smoothness);

            // 获取当前帧的视场角（FOV）比例。FOV越大，能容忍的抖动越多
            let frame = crate::frame_at_timestamp(*ts as f64 / 1000.0, compute_params.scaled_fps) as usize;
            let mut fov_ratio = if compute_params.camera_diagonal_fovs.len() == 1 {
                compute_params.camera_diagonal_fovs[0] / FOV_REFERENCE
            } else {
                compute_params.camera_diagonal_fovs.get(frame).map(|x| *x / FOV_REFERENCE).unwrap_or(1.0)
            };

            if let Some(fov_limit_ratio) = compute_params.smoothing_fov_limit_per_frame.get(frame) {
                fov_ratio *= *fov_limit_ratio;
            }

            // Calculate max velocity
            // 计算“最大允许速度”。这个速度由平滑度设置和FOV共同决定
            let mut max_velocity = [MAX_VELOCITY, MAX_VELOCITY, MAX_VELOCITY];
            if self.per_axis {
                max_velocity[0] *= smoothness_pitch * fov_ratio;
                max_velocity[1] *= smoothness_yaw   * fov_ratio;
                max_velocity[2] *= smoothness_roll  * fov_ratio;
            } else {
                max_velocity[0] *= smoothness * fov_ratio;
            }

            // Doing this to get similar max zoom as without second pass
            // 如果启用了第二遍平滑，将最大速度减半，以补偿额外的平滑效果
            if self.second_pass {
                max_velocity[0] *= 0.5;
                if self.per_axis {
                    max_velocity[1] *= 0.5;
                    max_velocity[2] *= 0.5;
                }
            }

            // 将实际速度除以最大允许速度，得到一个0-1的比例值
            // 抖动（低速）会得到一个接近0的值，而快速的有意图运动会得到一个接近1的值
            vel[0] /= max_velocity[0];
            if self.per_axis {
                vel[1] /= max_velocity[1];
                vel[2] /= max_velocity[2];
            }
        }

        // Plain 3D smoothing with varying alpha
        // Forward pass
        // 这是主要的平滑过程，它使用上一步计算的归一化速度来动态调整平滑强度
        // 正向传递
        let mut q = *quats.iter().next().unwrap().1; // 从第一个四元数开始
        let smoothed1: TimeQuat = quats.iter().map(|(ts, x)| {
            let ratio = velocity[ts]; // 获取当前时间的归一化速度
            // 获取当前时间的alpha值
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(ts).unwrap_or(&alpha_0_1s);
            if self.per_axis {
                // 根据速度比例，在“强平滑”和“弱平滑”之间进行插值，为每个轴计算一个最终的平滑因子
                let pitch_factor = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - ratio[1]) + alpha_0_1s * ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - ratio[2]) + alpha_0_1s * ratio[2];

                // 计算当前姿态q和目标姿态x之间的旋转差异
                let euler_rot = (q.inverse() * x).euler_angles();

                // 将这个差异按计算出的平滑因子进行缩放，然后应用到当前姿态上
                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                // 统一平滑：在“强平滑”和“弱平滑”之间插值得到一个单一的平滑因子
                let val = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q) // 返回新的时间戳和平滑后的姿态
        }).collect();

        // Reverse pass
        // 对smoothed1的结果进行一次反向的、完全相同的平滑操作。
        // 这样可以消除正向传递带来的相位延迟，使得稳定后的画面运动轨迹与原始意图更贴合
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        let smoothed2: TimeQuat = smoothed1.into_iter().rev().map(|(ts, x)| {
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(&ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(&ts).unwrap_or(&alpha_0_1s);
            let ratio = velocity[&ts];
            if self.per_axis {
                let pitch_factor = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - ratio[1]) + alpha_0_1s * ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - ratio[2]) + alpha_0_1s * ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                q = q.slerp(&x, val.min(1.0));
            }
            (ts, q)
        }).collect();

        // 如果不启用第二遍平滑，此时就可以返回结果了
        if !self.second_pass {
            return smoothed2;
        }

        // Calculate distance
        // 这一遍的目的是进一步优化，特别是减少因过度稳定可能产生的“画面被拉近或推远（zoom）”的副作用
        // 它通过分析第一遍平滑的“修正量”来实现

        // 计算距离：计算原始数据和第一遍平滑后数据之间的旋转差异
        // 这个“距离”代表了在每个时间点上，稳定算法“修正”了多少
        let mut distance = BTreeMap::<i64, Vector3<f64>>::new();
        let mut max_distance = Vector3::from_element(0.0);
        for (ts, quat) in smoothed2.iter() {
            let dist = quats[ts].inverse() * quat;
            if self.per_axis {
                let euler = dist.euler_angles();
                distance.insert(*ts, Vector3::new(
                    euler.0.abs(),
                    euler.1.abs(),
                    euler.2.abs()
                ));
                if euler.0.abs() > max_distance[0] { max_distance[0] = euler.0.abs(); }
                if euler.1.abs() > max_distance[1] { max_distance[1] = euler.1.abs(); }
                if euler.2.abs() > max_distance[2] { max_distance[2] = euler.2.abs(); }
            } else {
                distance.insert(*ts, Vector3::from_element(dist.angle()));
                if dist.angle() > max_distance[0] { max_distance[0] = dist.angle(); }
            }
        }

        // Normalize distance and discard under 0.5
        // 归一化距离并丢弃小值：将距离归一化，并把小于0.5的值视为0。
        // 这是为了只关注那些被“大幅修正”的片段
        for (_ts, dist) in distance.iter_mut() {
            dist[0] /= max_distance[0];
            if dist[0] < 0.5 { dist[0] = 0.0; }
            if self.per_axis {
                dist[1] /= max_distance[1];
                if dist[1] < 0.5 { dist[1] = 0.0; }
                dist[2] /= max_distance[2];
                if dist[2] < 0.5 { dist[2] = 0.0; }
            }
        }

        // Smooth distance
        // 平滑距离：对处理后的距离值本身也进行一次平滑，避免突变
        let mut prev_dist = *distance.iter().next().unwrap().1;
        for (_timestamp, dist) in distance.iter_mut().skip(1) {
            *dist = prev_dist * (1.0 - alpha_0_1s) + *dist * alpha_0_1s;
            prev_dist = *dist;
        }
        for (_timestamp, dist) in distance.iter_mut().rev().skip(1) {
            *dist = prev_dist * (1.0 - alpha_0_1s) + *dist * alpha_0_1s;
            prev_dist = *dist;
        }

        // Get max distance
        // 再次找到最大值并归一化，然后将范围映射到0.5-1.0
        // 最终得到的 `dist_ratio` 是一个调节因子
        max_distance = Vector3::from_element(0.0);
        for (_ts, dist) in distance.iter_mut() {
            if dist[0] > max_distance[0] { max_distance[0] = dist[0]; }
            if self.per_axis {
                if dist[1] > max_distance[1] { max_distance[1] = dist[1]; }
                if dist[2] > max_distance[2] { max_distance[2] = dist[2]; }
            }
        }

        // Normalize distance and change range to 0.5 - 1.0
        for (_ts, dist) in distance.iter_mut() {
            dist[0] /= max_distance[0];
            dist[0] = (dist[0] + 1.0) / 2.0;
            if self.per_axis {
                dist[1] /= max_distance[1];
                dist[1] = (dist[1] + 1.0) / 2.0;
                dist[2] /= max_distance[2];
                dist[2] = (dist[2] + 1.0) / 2.0;
            }
        }

        // Plain 3D smoothing with varying alpha
        // Forward pass
        // 第二遍平滑应用：正向传递
        // 这一遍与第一遍非常相似，但关键区别在于平滑因子的计算
        let mut q = *smoothed2.iter().next().unwrap().1;
        let smoothed1: TimeQuat = smoothed2.into_iter().map(|(ts, x)| {
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(&ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(&ts).unwrap_or(&alpha_0_1s);
            let vel_ratio = velocity[&ts]; // 第一遍计算的速度比例
            let dist_ratio = distance[&ts]; // 第二遍计算的距离比例
            if self.per_axis {
                // 注意这里的`vel_ratio * dist_ratio`，`dist_ratio`在这里作为一个调节器，微调 `vel_ratio` 的效果，
                // 从而对平滑强度进行更精细的二次控制
                let pitch_factor = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - vel_ratio[1] * dist_ratio[1]) + alpha_0_1s * vel_ratio[1] * dist_ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - vel_ratio[2] * dist_ratio[2]) + alpha_0_1s * vel_ratio[2] * dist_ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                q = q.slerp(&x, val.min(1.0));
            }
            (ts, q)
        }).collect();

        // Reverse pass
        // 第二遍平滑应用：反向传递
        // 同样，进行一次反向传递以消除相位延迟，并返回最终结果
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        smoothed1.into_iter().rev().map(|(ts, x)| {
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(&ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(&ts).unwrap_or(&alpha_0_1s);
            let vel_ratio = velocity[&ts];
            let dist_ratio = distance[&ts];
            if self.per_axis {
                let pitch_factor = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - vel_ratio[1] * dist_ratio[1]) + alpha_0_1s * vel_ratio[1] * dist_ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - vel_ratio[2] * dist_ratio[2]) + alpha_0_1s * vel_ratio[2] * dist_ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                q = q.slerp(&x, val.min(1.0));
            }
            (ts, q)
        }).collect()
    }
}