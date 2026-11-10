//! C functions not covered (or not conveniently) by the binding crates.

use std::ffi::c_void;

use core_foundation::base::{CFTypeID, CFTypeRef};
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::CFStringRef;

pub type AXUIElementRef = CFTypeRef;
pub type AXError = i32;
pub const AX_SUCCESS: AXError = 0;
/// `kAXValueTypeCGRect`.
pub const AX_VALUE_CG_RECT: u32 = 3;
/// `kAXValueTypeCFRange`.
pub const AX_VALUE_CF_RANGE: u32 = 4;

/// A CoreGraphics rectangle (origin and size in points).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct CgRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A CoreFoundation range: `CFIndex` location and length.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct CfRange {
    pub location: isize,
    pub length: isize,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    pub fn AXUIElementGetTypeID() -> CFTypeID;
    pub fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    pub fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXError;
    pub fn AXUIElementCopyElementAtPosition(
        application: AXUIElementRef,
        x: f32,
        y: f32,
        element: *mut AXUIElementRef,
    ) -> AXError;
    pub fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> AXError;
    pub fn AXIsProcessTrusted() -> bool;
    pub fn AXValueGetValue(value: CFTypeRef, value_type: u32, value_ptr: *mut c_void) -> bool;
    pub fn AXValueCreate(value_type: u32, value_ptr: *const c_void) -> CFTypeRef;
    pub fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    pub fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    pub static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    /// Current modifier flags; `state_id` 1 = HID system state (the
    /// physical keyboard).
    pub fn CGEventSourceFlagsState(state_id: i32) -> u64;
    /// Whether a key (a virtual key code) is down; `state_id` 0 = the
    /// login session's combined state (the keyboard and what apps post).
    pub fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    /// The same for a mouse button, numbered as CGEvent counts them.
    pub fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
}

pub type TISInputSourceRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    /// Returns a `Boolean` (u8).
    pub fn IsSecureEventInputEnabled() -> u8;

    // Text Input Sources: main thread only on recent macOS.
    pub fn TISCopyCurrentKeyboardLayoutInputSource() -> TISInputSourceRef;
    pub fn TISGetInputSourceProperty(source: TISInputSourceRef, key: CFStringRef) -> *const c_void;
    pub static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    pub fn LMGetKbdType() -> u8;
    #[allow(clippy::too_many_arguments)]
    pub fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: u32,
        dead_key_state: *mut u32,
        max_string_length: usize,
        actual_string_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
}
