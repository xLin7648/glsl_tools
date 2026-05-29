use std::ffi::{c_char, CStr, CString};
use std::ptr;

use glslang::{
    Compiler, CompilerOptions, GlslProfile, Shader, ShaderInput, ShaderMessage,
    ShaderOptions, ShaderSource, SourceLanguage, SpirvVersion, Target, VulkanVersion,
};

// ============================================================
//  常量
// ============================================================

const VULKAN_VERSION: VulkanVersion = VulkanVersion::Vulkan1_0;
const SPIRV_VERSION: SpirvVersion = SpirvVersion::SPIRV1_0;

// ============================================================
//  内部辅助 — 编译管线
// ============================================================

fn acquire_compiler() -> Result<&'static Compiler, String> {
    Compiler::acquire().ok_or_else(|| "无法初始化 glslang 编译器".to_string())
}

fn make_vulkan_target() -> Target {
    Target::Vulkan {
        version: VULKAN_VERSION,
        spirv_version: SPIRV_VERSION,
    }
}

fn make_options(target: Target) -> CompilerOptions {
    CompilerOptions {
        source_language: SourceLanguage::GLSL,
        target,
        version_profile: None,
        messages: ShaderMessage::DEFAULT,
    }
}

fn make_shader_input<'a>(
    source: &'a ShaderSource,
    options: &'a CompilerOptions,
) -> Result<ShaderInput<'a>, glslang::error::GlslangError> {
    ShaderInput::new(
        source,
        glslang::ShaderStage::Fragment,
        options,
        None::<&[(&str, Option<&str>)]>,
        None,
    )
}

// ============================================================
//  Rust 公开 API
// ============================================================

/// 将 GLSL 源代码编译为 SPIR-V 字节码
///
/// * `source` - GLSL 片元着色器源码文本
///
/// 成功时返回 SPIR-V 字节码 (`Vec<u8>`)
/// 失败时返回错误描述 (`String`)
pub fn convert_glsl_to_spv(source: &str) -> Result<Vec<u8>, String> {
    let compiler = acquire_compiler()?;
    let source = ShaderSource::from(source);
    let options = make_options(make_vulkan_target());

    let input = make_shader_input(&source, &options)
        .map_err(|e| format!("创建 ShaderInput 失败: {e}"))?;

    let mut shader =
        Shader::new(compiler, input).map_err(|e| format!("创建 Shader 失败: {e}"))?;
    shader.options(ShaderOptions::AUTO_MAP_LOCATIONS);

    let spv_words = shader
        .compile()
        .map_err(|e| format!("编译失败: {e}"))?;

    Ok(bytemuck::cast_slice(&spv_words).to_vec())
}

// ============================================================
//  内部辅助 — FFI
// ============================================================

/// 将错误信息写入堆分配的 C 字符串，通过 out_error 返回
fn set_error(out_error: *mut *mut c_char, message: &str) {
    let c_string = match CString::new(message) {
        Ok(cs) => cs,
        Err(_) => CString::new("未知错误（含内嵌 null 字符）").unwrap(),
    };
    let ptr = c_string.into_raw(); // 移交所有权给调用方
    unsafe {
        *out_error = ptr;
    }
}

/// 释放由 FFI 函数分配的 CString 类型错误字符串
fn free_error_string(error: *mut c_char) {
    if !error.is_null() {
        unsafe {
            drop(CString::from_raw(error));
        }
    }
}

// ============================================================
//  FFI — to_spirv (编译 GLSL → SPIR-V)
// ============================================================

/// 编译 GLSL 源代码为 SPIR-V 字节码
///
/// # 参数
/// - `source`: 以 null 结尾的 GLSL 源码字符串 (in)
/// - `out_spv_data`: 接收 SPIR-V 数据的指针 (out, 由 `free_spv` 释放)
/// - `out_spv_len`: 接收 SPIR-V 数据长度（字节数）(out)
/// - `out_error`: 接收错误字符串指针 (out, 由 `free_error` 释放)
///
/// # 返回值
/// - `0`: 成功, `out_spv_data` / `out_spv_len` 有效, `out_error` 为 null
/// - `-1`: 失败, `out_error` 包含错误描述
#[unsafe(no_mangle)]
pub extern "C" fn to_spirv(
    source: *const c_char,
    out_spv_data: *mut *mut u8,
    out_spv_len: *mut i32,
    out_error: *mut *mut c_char,
) -> i32 {
    if source.is_null() || out_spv_data.is_null() || out_spv_len.is_null() || out_error.is_null() {
        return -1;
    }

    unsafe {
        *out_spv_data = ptr::null_mut();
        *out_spv_len = 0;
        *out_error = ptr::null_mut();
    }

    let source_str = match unsafe { CStr::from_ptr(source) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_error(out_error, &format!("无效的 UTF-8 输入: {e}"));
            return -1;
        }
    };

    match convert_glsl_to_spv(source_str) {
        Ok(spv_bytes) => {
            let boxed_slice = spv_bytes.into_boxed_slice();
            let ptr = boxed_slice.as_ptr() as *mut u8;
            let len = boxed_slice.len();
            std::mem::forget(boxed_slice); // 将所有权交给调用方

            unsafe {
                *out_spv_data = ptr;
                *out_spv_len = len as i32;
            }
            0
        }
        Err(e) => {
            set_error(out_error, &e);
            -1
        }
    }
}

/// 释放由 `to_spirv` 分配的 SPIR-V 数据
#[unsafe(no_mangle)]
pub extern "C" fn free_spv(spv_data: *mut u8, spv_len: i32) {
    if spv_data.is_null() || spv_len <= 0 {
        return;
    }
    unsafe {
        let slice = std::ptr::slice_from_raw_parts_mut(spv_data, spv_len as usize);
        let _ = Box::from_raw(slice);
    }
}

/// 检测 GLSL 源码是否使用 ES 版本指令（如 `#version 320 es`）
fn source_is_es(source: &str) -> bool {
    source.lines().any(|line| {
        let t = line.trim();
        // 匹配 "#version XXX es" 模式
        if let Some(rest) = t.strip_prefix("#version") {
            rest.trim_end().ends_with("es")
        } else {
            false
        }
    })
}

/// 从 GLSL 源码解析 `#version` 指令中的版本号
fn parse_glsl_version(source: &str) -> Option<i32> {
    for line in source.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("#version") {
            let num_str = rest.split_whitespace().next()?;
            return num_str.parse::<i32>().ok();
        }
    }
    None
}

// ============================================================
//  FFI — verify (验证 GLSL 语法)
// ============================================================

/// 验证 GLSL 片元着色器源码是否正确。
///
/// 参数:
///   source    - 以 null 结尾的 GLSL 源码字符串
///   is_vulkan - true 使用 Vulkan 语义规则，false 使用 OpenGL 规则
///   error_out - 输出错误信息字符串（失败时分配，由 `free_error` 释放）
///
/// 返回: 0 成功，-1失败
#[unsafe(no_mangle)]
pub unsafe extern "C" fn verify(
    source: *const c_char,
    is_vulkan: bool,
    error_out: *mut *mut c_char,
) -> i32 {
    if source.is_null() || error_out.is_null() {
        return -1;
    }
    unsafe { *error_out = std::ptr::null_mut() };

    let source_str = match unsafe { CStr::from_ptr(source) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let compiler = match acquire_compiler() {
        Ok(c) => c,
        Err(e) => {
            set_error(error_out, &e);
            return -1;
        }
    };

    let (target, version_profile) = if is_vulkan {
        (make_vulkan_target(), None)
    } else {
        // 非 Vulkan 着色器（ES 或 Core）：只做 parse 语法检查，不进入
        // compile()。该 crate 的 program.compile() 始终施加 SPIR-V 规则
        // （强制 location、non-opaque uniform 必须入 block / 带 location），
        // 这些规则对桌面 OpenGL 和 GLES 的常规验证不适用。
        let version_profile = if source_is_es(source_str) {
            let version = parse_glsl_version(source_str).unwrap_or(300);
            Some((version, GlslProfile::ES))
        } else {
            None // 桌面 GLSL 由 glslang 从 #version 450 自动检测
        };
        (Target::None(None), version_profile)
    };

    let options = CompilerOptions {
        source_language: SourceLanguage::GLSL,
        target,
        version_profile,
        messages: ShaderMessage::DEFAULT,
    };
    let glsl_source = ShaderSource::from(source_str);

    let input = match make_shader_input(&glsl_source, &options) {
        Ok(i) => i,
        Err(e) => {
            set_error(error_out, &format!("{e}"));
            return -1;
        }
    };

    let mut shader = match Shader::new(compiler, input) {
        Ok(s) => s,
        Err(e) => {
            set_error(error_out, &format!("{e}"));
            return -1;
        }
    };

    if !is_vulkan {
        // Shader::new() parse 成功 = 语法有效
        return 0;
    }

    shader.options(ShaderOptions::AUTO_MAP_LOCATIONS);

    match shader.compile() {
        Ok(_) => 0,
        Err(e) => {
            set_error(error_out, &format!("语法错误: {e}"));
            -1
        }
    }
}

// ============================================================
//  FFI — 共享内存管理
// ============================================================

/// 释放由 `to_spirv` 或 `verify` 分配的错误字符串
#[unsafe(no_mangle)]
pub extern "C" fn free_error(error: *mut c_char) {
    free_error_string(error);
}
