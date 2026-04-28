use cssimpler_core::Color;
use winit::window::Window;

pub(crate) fn apply(window: &Window, tint: Color) -> Result<bool, String> {
    platform::apply(window, tint)
}

pub(crate) fn clear(window: &Window) -> Result<(), String> {
    platform::clear(window)
}

pub(crate) fn requires_initial_transparency() -> bool {
    platform::requires_initial_transparency()
}

pub(crate) fn uses_custom_presenter() -> bool {
    platform::uses_custom_presenter()
}

pub(crate) fn present(
    window: &Window,
    buffer: &[u32],
    width: usize,
    height: usize,
    scale_factor: f64,
) -> Result<bool, String> {
    platform::present(window, buffer, width, height, scale_factor)
}

#[cfg(target_os = "windows")]
mod platform {
    use cssimpler_core::Color;
    use winit::window::Window;

    pub(super) fn apply(window: &Window, tint: Color) -> Result<bool, String> {
        apply_accent(window, ACCENT_ENABLE_ACRYLICBLURBEHIND, tint)?;
        Ok(true)
    }

    pub(super) fn clear(window: &Window) -> Result<(), String> {
        apply_accent(window, ACCENT_DISABLED, Color::rgba(0, 0, 0, 0))
    }

    pub(super) fn requires_initial_transparency() -> bool {
        true
    }

    pub(super) fn uses_custom_presenter() -> bool {
        false
    }

    pub(super) fn present(
        _window: &Window,
        _buffer: &[u32],
        _width: usize,
        _height: usize,
        _scale_factor: f64,
    ) -> Result<bool, String> {
        Ok(false)
    }

    const ACCENT_DISABLED: u32 = 0;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

    fn apply_accent(window: &Window, accent_state: u32, tint: Color) -> Result<(), String> {
        use std::ffi::c_void;
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
        use windows_sys::core::BOOL;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        type SetWindowCompositionAttribute =
            unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;

        #[repr(C)]
        struct AccentPolicy {
            accent_state: u32,
            accent_flags: u32,
            gradient_color: u32,
            animation_id: u32,
        }

        #[repr(C)]
        struct WindowCompositionAttribData {
            attrib: u32,
            pv_data: *mut c_void,
            cb_data: usize,
        }

        const WCA_ACCENT_POLICY: u32 = 0x13;

        let raw = window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let hwnd = match raw {
            RawWindowHandle::Win32(handle) => handle.hwnd.get() as HWND,
            _ => return Err("window is not a Win32 window".to_string()),
        };

        let user32 = unsafe { LoadLibraryA(c"user32.dll".as_ptr() as _) };
        if user32.is_null() {
            return Err("failed to load user32.dll".to_string());
        }

        let proc =
            unsafe { GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr() as _) }
                .ok_or_else(|| "SetWindowCompositionAttribute is unavailable".to_string())?;
        let set_window_composition_attribute: SetWindowCompositionAttribute =
            unsafe { std::mem::transmute(proc) };
        let mut policy = AccentPolicy {
            accent_state,
            accent_flags: 0,
            gradient_color: packed_windows_tint(tint),
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            pv_data: &mut policy as *mut _ as _,
            cb_data: std::mem::size_of::<AccentPolicy>(),
        };

        let ok = unsafe { set_window_composition_attribute(hwnd, &mut data) };
        if ok == 0 {
            return Err("SetWindowCompositionAttribute failed".to_string());
        }

        Ok(())
    }

    fn packed_windows_tint(tint: Color) -> u32 {
        u32::from(tint.r)
            | (u32::from(tint.g) << 8)
            | (u32::from(tint.b) << 16)
            | (u32::from(tint.a) << 24)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::{self, NonNull, slice_from_raw_parts_mut};

    use cssimpler_core::Color;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{MainThreadMarker, msg_send};
    use objc2_core_foundation::{CFRetained, CGFloat, CGPoint, CGRect};
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
        CGImageAlphaInfo, CGImageByteOrderInfo, CGImageComponentInfo, CGImagePixelFormatInfo,
    };
    use objc2_foundation::{NSInteger, NSUInteger, ns_string};
    use objc2_quartz_core::{CALayer, CATransaction, kCAGravityResize};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use crate::{is_transparent, to_softbuffer_rgb_blue_noise, unpack_alpha8};

    const NS_VIEW_WIDTH_SIZABLE: NSUInteger = 2;
    const NS_VIEW_HEIGHT_SIZABLE: NSUInteger = 16;
    const NS_VISUAL_EFFECT_BLENDING_BEHIND_WINDOW: NSInteger = 0;
    const NS_VISUAL_EFFECT_MATERIAL_HUD_WINDOW: NSInteger = 13;
    const NS_VISUAL_EFFECT_STATE_ACTIVE: NSInteger = 1;

    thread_local! {
        static STATES: RefCell<HashMap<usize, MacGlassState>> = RefCell::new(HashMap::new());
    }

    struct MacGlassState {
        view: Retained<AnyObject>,
        view_alpha: CGFloat,
        effect_view: Retained<AnyObject>,
        root_layer: Retained<CALayer>,
        root_layer_was_opaque: bool,
        tint_layer: Retained<CALayer>,
        overlay_layer: Retained<CALayer>,
        hidden_sublayers: Vec<Retained<CALayer>>,
    }

    pub(super) fn apply(window: &Window, tint: Color) -> Result<bool, String> {
        let view_ptr = appkit_view_ptr(window)?;
        let ns_window = appkit_window_for_view_ptr(view_ptr)?;
        let key = object_key(&ns_window);

        STATES.with(|states| -> Result<(), String> {
            let mut states = states.borrow_mut();
            if let Some(state) = states.get(&key) {
                configure_effect_view(&state.effect_view, tint);
                return Ok(());
            }

            let state = install(window, view_ptr, ns_window, tint)?;
            states.insert(key, state);
            Ok(())
        })?;

        Ok(true)
    }

    pub(super) fn clear(window: &Window) -> Result<(), String> {
        let view_ptr = appkit_view_ptr(window)?;
        let ns_window = appkit_window_for_view_ptr(view_ptr)?;
        let key = object_key(&ns_window);
        STATES.with(|states| {
            if let Some(state) = states.borrow_mut().remove(&key) {
                restore_hidden_sublayers(&state.hidden_sublayers);
                state.root_layer.setOpaque(state.root_layer_was_opaque);
                let _: () = unsafe { msg_send![&*state.view, setAlphaValue: state.view_alpha] };
                state.tint_layer.removeFromSuperlayer();
                state.overlay_layer.removeFromSuperlayer();
                let _: () = unsafe { msg_send![&*state.view, removeFromSuperview] };
                let _: () = unsafe { msg_send![&*ns_window, setContentView: Some(&*state.view)] };
            }
        });
        Ok(())
    }

    pub(super) fn requires_initial_transparency() -> bool {
        true
    }

    pub(super) fn uses_custom_presenter() -> bool {
        true
    }

    pub(super) fn present(
        window: &Window,
        buffer: &[u32],
        width: usize,
        height: usize,
        scale_factor: f64,
    ) -> Result<bool, String> {
        let view_ptr = appkit_view_ptr(window)?;
        let ns_window = appkit_window_for_view_ptr(view_ptr)?;
        let key = object_key(&ns_window);
        STATES.with(|states| {
            let mut states = states.borrow_mut();
            let state = states
                .get_mut(&key)
                .ok_or_else(|| "macOS native glass presenter was not installed".to_string())?;
            state.present(buffer, width, height, scale_factor)
        })?;
        Ok(true)
    }

    fn appkit_view_ptr(window: &Window) -> Result<*mut AnyObject, String> {
        let raw = window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        match raw {
            RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr().cast::<AnyObject>()),
            _ => Err("window is not an AppKit window".to_string()),
        }
    }

    fn appkit_window_for_view_ptr(view_ptr: *mut AnyObject) -> Result<Retained<AnyObject>, String> {
        let view = unsafe { view_ptr.as_ref() }
            .ok_or_else(|| "AppKit view handle was null".to_string())?;
        let ns_window: Option<Retained<AnyObject>> = unsafe { msg_send![view, window] };
        ns_window.ok_or_else(|| "AppKit view is not attached to a window".to_string())
    }

    fn object_key(object: &AnyObject) -> usize {
        object as *const AnyObject as usize
    }

    fn install(
        window: &Window,
        view_ptr: *mut AnyObject,
        ns_window: Retained<AnyObject>,
        tint: Color,
    ) -> Result<MacGlassState, String> {
        let _main_thread = MainThreadMarker::new().ok_or_else(|| {
            "AppKit native glass must be installed on the main thread".to_string()
        })?;
        let view = unsafe { view_ptr.as_ref() }
            .ok_or_else(|| "AppKit view handle was null".to_string())?;
        let view = unsafe { Retained::retain(view as *const _ as *mut AnyObject) }
            .ok_or_else(|| "failed to retain AppKit view".to_string())?;
        let view_alpha: CGFloat = unsafe { msg_send![&*view, alphaValue] };

        let frame: CGRect = unsafe { msg_send![&*view, frame] };
        let effect_view = new_visual_effect_view(frame, tint)?;
        let clear_color = ns_color("clearColor")?;

        let _: () = unsafe { msg_send![&*ns_window, setOpaque: Bool::NO] };
        let _: () = unsafe { msg_send![&*ns_window, setBackgroundColor: Some(&*clear_color)] };
        let _: () = unsafe { msg_send![&*ns_window, setContentView: Some(&*effect_view)] };

        let bounds: CGRect = unsafe { msg_send![&*effect_view, bounds] };
        let _: () = unsafe { msg_send![&*view, setFrame: bounds] };
        let _: () = unsafe {
            msg_send![
                &*view,
                setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE
            ]
        };
        let _: () = unsafe { msg_send![&*effect_view, addSubview: &*view] };
        let _: () = unsafe { msg_send![&*view, setWantsLayer: Bool::YES] };

        let root_layer: Option<Retained<CALayer>> = unsafe { msg_send![&*view, layer] };
        let root_layer =
            root_layer.ok_or_else(|| "failed to create AppKit view backing layer".to_string())?;
        let root_layer_was_opaque = root_layer.isOpaque();
        root_layer.setOpaque(false);
        let hidden_sublayers = hide_existing_sublayers(&root_layer);

        let effect_layer: Option<Retained<CALayer>> = unsafe { msg_send![&*effect_view, layer] };
        let effect_layer = effect_layer
            .ok_or_else(|| "failed to create native glass effect backing layer".to_string())?;
        effect_layer.setOpaque(false);

        let tint_layer = new_presenter_layer(
            ns_string!("cssimpler.native_glass.tint"),
            bounds,
            999_999.0,
            window.scale_factor(),
        );
        let overlay_layer = new_presenter_layer(
            ns_string!("cssimpler.native_glass.overlay"),
            bounds,
            1_000_000.0,
            window.scale_factor(),
        );
        effect_layer.addSublayer(&tint_layer);
        effect_layer.addSublayer(&overlay_layer);

        let _: () = unsafe { msg_send![&*view, setAlphaValue: 0.0 as CGFloat] };

        Ok(MacGlassState {
            view,
            view_alpha,
            effect_view,
            root_layer,
            root_layer_was_opaque,
            tint_layer,
            overlay_layer,
            hidden_sublayers,
        })
    }

    fn new_presenter_layer(
        name: &objc2_foundation::NSString,
        bounds: CGRect,
        z_position: f64,
        scale_factor: f64,
    ) -> Retained<CALayer> {
        let layer = CALayer::new();
        layer.setName(Some(name));
        layer.setOpaque(false);
        layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
        layer.setGeometryFlipped(true);
        layer.setContentsGravity(unsafe { kCAGravityResize });
        layer.setFrame(bounds);
        layer.setZPosition(z_position);
        layer.setContentsScale(scale_factor);
        layer
    }

    fn hide_existing_sublayers(root_layer: &CALayer) -> Vec<Retained<CALayer>> {
        let Some(sublayers) = (unsafe { root_layer.sublayers() }) else {
            return Vec::new();
        };

        let mut hidden_sublayers = Vec::new();
        for index in 0..sublayers.len() {
            let layer = sublayers.objectAtIndex(index);
            if layer.isHidden() {
                continue;
            }
            layer.setHidden(true);
            hidden_sublayers.push(layer);
        }
        hidden_sublayers
    }

    fn restore_hidden_sublayers(sublayers: &[Retained<CALayer>]) {
        for layer in sublayers {
            layer.setHidden(false);
        }
    }

    fn new_visual_effect_view(frame: CGRect, tint: Color) -> Result<Retained<AnyObject>, String> {
        let class = AnyClass::get(c"NSVisualEffectView")
            .ok_or_else(|| "NSVisualEffectView is unavailable".to_string())?;
        let effect_view: Retained<AnyObject> = unsafe { msg_send![class, new] };
        let _: () = unsafe { msg_send![&*effect_view, setFrame: frame] };
        let _: () = unsafe { msg_send![&*effect_view, setWantsLayer: Bool::YES] };
        let _: () = unsafe {
            msg_send![
                &*effect_view,
                setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE
            ]
        };
        configure_effect_view(&effect_view, tint);
        Ok(effect_view)
    }

    fn configure_effect_view(effect_view: &AnyObject, _tint: Color) {
        let _: () = unsafe {
            msg_send![
                effect_view,
                setMaterial: NS_VISUAL_EFFECT_MATERIAL_HUD_WINDOW
            ]
        };
        let _: () = unsafe {
            msg_send![
                effect_view,
                setBlendingMode: NS_VISUAL_EFFECT_BLENDING_BEHIND_WINDOW
            ]
        };
        let _: () = unsafe { msg_send![effect_view, setState: NS_VISUAL_EFFECT_STATE_ACTIVE] };
        let _: () = unsafe { msg_send![effect_view, setEmphasized: Bool::YES] };
    }

    fn ns_color(selector: &str) -> Result<Retained<AnyObject>, String> {
        let class =
            AnyClass::get(c"NSColor").ok_or_else(|| "NSColor is unavailable".to_string())?;
        match selector {
            "clearColor" => Ok(unsafe { msg_send![class, clearColor] }),
            _ => Err(format!("unsupported NSColor selector: {selector}")),
        }
    }

    impl MacGlassState {
        fn present(
            &mut self,
            buffer: &[u32],
            width: usize,
            height: usize,
            scale_factor: f64,
        ) -> Result<(), String> {
            if width == 0 || height == 0 {
                return Ok(());
            }
            if buffer.len() != width.saturating_mul(height) {
                return Err("native glass presenter received a mismatched buffer".to_string());
            }

            let bounds: CGRect = unsafe { msg_send![&*self.effect_view, bounds] };
            let content_image =
                create_alpha_image(buffer, width, height, AlphaImageLayer::Content)?;
            let tint_image = create_alpha_image(buffer, width, height, AlphaImageLayer::Tint)?;

            CATransaction::begin();
            CATransaction::setDisableActions(true);
            self.tint_layer.setFrame(bounds);
            self.tint_layer.setContentsScale(scale_factor);
            self.overlay_layer.setFrame(bounds);
            self.overlay_layer.setContentsScale(scale_factor);
            unsafe { self.tint_layer.setContents(Some(tint_image.as_ref())) };
            unsafe { self.overlay_layer.setContents(Some(content_image.as_ref())) };
            CATransaction::commit();

            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    enum AlphaImageLayer {
        Content,
        Tint,
    }

    fn create_alpha_image(
        buffer: &[u32],
        width: usize,
        height: usize,
        layer: AlphaImageLayer,
    ) -> Result<CFRetained<CGImage>, String> {
        unsafe extern "C-unwind" fn release(
            _info: *mut c_void,
            data: NonNull<c_void>,
            size: usize,
        ) {
            let data = data.cast::<u32>();
            let slice = slice_from_raw_parts_mut(data.as_ptr(), size / size_of::<u32>());
            drop(unsafe { Box::from_raw(slice) });
        }

        let mut pixels = Vec::with_capacity(buffer.len());
        for row in 0..height {
            for column in 0..width {
                let pixel = buffer[row * width + column];
                if is_transparent(pixel) {
                    pixels.push(0);
                } else {
                    let alpha = u32::from(unpack_alpha8(pixel));
                    if alpha < 255 && matches!(layer, AlphaImageLayer::Content) {
                        pixels.push(0);
                        continue;
                    }
                    if alpha == 255 && matches!(layer, AlphaImageLayer::Tint) {
                        pixels.push(0);
                        continue;
                    }
                    let rgb = to_softbuffer_rgb_blue_noise(pixel, column, row);
                    let red = ((rgb >> 16) & 0xff) * alpha / 255;
                    let green = ((rgb >> 8) & 0xff) * alpha / 255;
                    let blue = (rgb & 0xff) * alpha / 255;
                    pixels.push((alpha << 24) | (red << 16) | (green << 8) | blue);
                }
            }
        }

        let data_provider = {
            let len = pixels.len() * size_of::<u32>();
            let buffer: *mut [u32] = Box::into_raw(pixels.into_boxed_slice());
            let data_ptr = buffer.cast::<c_void>();
            unsafe {
                CGDataProvider::with_data(ptr::null_mut(), data_ptr, len, Some(release))
                    .ok_or_else(|| "failed to create native glass data provider".to_string())?
            }
        };

        let color_space = CGColorSpace::new_device_rgb()
            .ok_or_else(|| "failed to create native glass color space".to_string())?;
        let bitmap_info = CGBitmapInfo(
            CGImageAlphaInfo::PremultipliedFirst.0
                | CGImageComponentInfo::Integer.0
                | CGImageByteOrderInfo::Order32Little.0
                | CGImagePixelFormatInfo::Packed.0,
        );

        unsafe {
            CGImage::new(
                width,
                height,
                8,
                32,
                width * 4,
                Some(&color_space),
                bitmap_info,
                Some(&data_provider),
                ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .ok_or_else(|| "failed to create native glass image".to_string())
    }
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
mod platform {
    use cssimpler_core::Color;
    use winit::window::Window;

    pub(super) fn apply(_window: &Window, _tint: Color) -> Result<bool, String> {
        Ok(false)
    }

    pub(super) fn clear(_window: &Window) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn requires_initial_transparency() -> bool {
        false
    }

    pub(super) fn uses_custom_presenter() -> bool {
        false
    }

    pub(super) fn present(
        _window: &Window,
        _buffer: &[u32],
        _width: usize,
        _height: usize,
        _scale_factor: f64,
    ) -> Result<bool, String> {
        Ok(false)
    }
}
