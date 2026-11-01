// Powermove modified: native S2 interop; see vendor/gpui-ce/POWERMOVE.md.
#[cfg(target_os = "windows")]
use crate::WindowsScreenCaptureFrame;
use crate::{
    App, Bounds, DevicePixels, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, Size, Style, StyleRefinement, Styled, Window,
};
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::CVPixelBuffer;
use refineable::Refineable;

/// A source of a surface's content.
#[derive(Clone)]
pub enum SurfaceSource {
    /// A retained, fenced, packed RGB Metal texture.
    #[cfg(target_os = "macos")]
    NativeTexture(std::sync::Arc<NativeTexture>),
    /// A macOS image buffer from CoreVideo
    #[cfg(target_os = "macos")]
    Surface(CVPixelBuffer),
    /// A GPU texture handle (type-erased to avoid depending on wgpu)
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        all(target_os = "macos", feature = "custom-gpu"),
        all(target_family = "wasm", feature = "custom-gpu")
    ))]
    Texture {
        /// The GPU texture, type-erased (expected to be `Arc<wgpu::Texture>`)
        #[cfg(not(target_family = "wasm"))]
        texture: std::sync::Arc<dyn std::any::Any + Send + Sync>,
        /// The GPU texture, type-erased (expected to be `Arc<wgpu::Texture>`).
        ///
        /// WGPU handles are intentionally thread-local in browser builds.
        #[cfg(target_family = "wasm")]
        texture: std::sync::Arc<dyn std::any::Any>,
        /// Dimensions of the texture in device pixels
        size: Size<DevicePixels>,
    },
    /// A native Windows Graphics Capture texture.
    #[cfg(target_os = "windows")]
    WindowsCapture(WindowsScreenCaptureFrame),
    /// A placeholder for platforms that cannot import native surfaces.
    #[doc(hidden)]
    Unsupported(Size<DevicePixels>),
}

impl std::fmt::Debug for SurfaceSource {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            #[cfg(target_os = "macos")]
            SurfaceSource::NativeTexture(ref texture) => _f
                .debug_tuple("NativeTexture")
                .field(&texture.texture.width())
                .finish(),
            #[cfg(target_os = "macos")]
            SurfaceSource::Surface(ref buf) => _f.debug_tuple("Surface").field(buf).finish(),
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                all(target_os = "macos", feature = "custom-gpu"),
                all(target_family = "wasm", feature = "custom-gpu")
            ))]
            SurfaceSource::Texture { size, .. } => _f
                .debug_struct("Texture")
                .field("size", &size)
                .finish_non_exhaustive(),
            #[cfg(target_os = "windows")]
            SurfaceSource::WindowsCapture(ref frame) => frame.fmt(_f),
            SurfaceSource::Unsupported(size) => _f.debug_tuple("Unsupported").field(&size).finish(),
        }
    }
}

impl SurfaceSource {
    fn size(&self) -> Size<DevicePixels> {
        match self {
            #[cfg(target_os = "macos")]
            SurfaceSource::NativeTexture(texture) => crate::size(
                (texture.texture.width() as i32).into(),
                (texture.texture.height() as i32).into(),
            ),
            #[cfg(target_os = "macos")]
            SurfaceSource::Surface(buffer) => {
                crate::size(buffer.get_width().into(), buffer.get_height().into())
            }
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                all(target_os = "macos", feature = "custom-gpu"),
                all(target_family = "wasm", feature = "custom-gpu")
            ))]
            SurfaceSource::Texture { size, .. } => *size,
            #[cfg(target_os = "windows")]
            SurfaceSource::WindowsCapture(frame) => frame.size(),
            SurfaceSource::Unsupported(size) => *size,
        }
    }
}

#[cfg(target_os = "macos")]
impl From<CVPixelBuffer> for SurfaceSource {
    fn from(value: CVPixelBuffer) -> Self {
        SurfaceSource::Surface(value)
    }
}

#[cfg(target_os = "windows")]
impl From<WindowsScreenCaptureFrame> for SurfaceSource {
    fn from(value: WindowsScreenCaptureFrame) -> Self {
        SurfaceSource::WindowsCapture(value)
    }
}

#[cfg(all(target_os = "windows", feature = "screen-capture"))]
impl From<crate::ScreenCaptureFrame> for SurfaceSource {
    fn from(value: crate::ScreenCaptureFrame) -> Self {
        SurfaceSource::WindowsCapture(value.0)
    }
}

/// A surface element.
pub struct Surface {
    source: SurfaceSource,
    object_fit: ObjectFit,
    style: StyleRefinement,
}

/// Create a new surface element.
pub fn surface(source: impl Into<SurfaceSource>) -> Surface {
    Surface {
        source: source.into(),
        object_fit: ObjectFit::Contain,
        style: Default::default(),
    }
}

impl Surface {
    /// Set the object fit for the image.
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = object_fit;
        self
    }
}

impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        _window: &mut Window,
        _: &mut App,
    ) {
        let new_bounds = self.object_fit.get_bounds(_bounds, self.source.size());
        // TODO: Add support for corner_radii.
        let mut style = Style::default();
        style.refine(&self.style);
        _window.with_element_opacity(style.opacity, |window| {
            window.paint_surface(new_bounds, self.source.clone());
        });
    }
}

impl IntoElement for Surface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Surface {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// Engine pool lease: scene clones and every GPU submission keep this alive.
#[cfg(target_os = "macos")]
pub trait TextureLease: std::fmt::Debug + Send + Sync {
    /// GPU sampling ended, not physical scanout. May run on a Metal thread.
    fn gpu_completed(&self, succeeded: bool);
}

/// Packed RGB source. Input is premultiplied, top-down, display-encoded sRGB.
/// RGBA16F is accepted only after the engine's extended-linear → SDR conversion.
/// The retained Metal objects bridge objc2-metal via +1 ownership, never borrowed raw pointers.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct NativeTexture {
    /// Retained source texture.
    pub texture: metal::Texture,
    /// Producer event, on the same device.
    pub event: metal::Event,
    /// Event value signalled by the producing command buffer.
    pub value: u64,
    /// Pool lifetime shared with every scene/submission.
    pub lease: std::sync::Arc<dyn TextureLease>,
    retired: std::sync::atomic::AtomicBool,
}
#[cfg(target_os = "macos")]
impl NativeTexture {
    /// Takes retained (+1) objects. Producer must signal value on its command queue,
    /// never mutate the texture while any scene/submission retains this lease, and
    /// use an event on the same device. No CPU wait is inserted.
    pub fn new(
        texture: metal::Texture,
        event: metal::Event,
        value: u64,
        lease: std::sync::Arc<dyn TextureLease>,
    ) -> Result<std::sync::Arc<Self>, &'static str> {
        if !matches!(
            texture.pixel_format(),
            metal::MTLPixelFormat::BGRA8Unorm | metal::MTLPixelFormat::RGBA16Float
        ) {
            return Err("P-TEXTURE requires BGRA8Unorm or converted RGBA16Float");
        }
        if texture.device().registry_id() != event.device().registry_id() {
            return Err("P-TEXTURE event/texture device mismatch");
        }
        Ok(std::sync::Arc::new(Self {
            texture,
            event,
            value,
            lease,
            retired: false.into(),
        }))
    }
    /// Retain borrowed objc2-metal pointers (+0 → +1). Does not transfer ownership.
    ///
    /// # Safety
    /// Both non-null pointers must be live MTLTexture/MTLEvent objects for this call.
    /// Producer obeys the same format, device, premultiplication and lease contract as `new`.
    pub unsafe fn from_borrowed_raw(
        texture: *mut std::ffi::c_void,
        event: *mut std::ffi::c_void,
        value: u64,
        lease: std::sync::Arc<dyn TextureLease>,
    ) -> Result<std::sync::Arc<Self>, &'static str> {
        use foreign_types::ForeignTypeRef;
        if texture.is_null() || event.is_null() {
            return Err("P-TEXTURE null Metal object");
        }
        let texture = unsafe { metal::TextureRef::from_ptr(texture.cast()) }.to_owned();
        let event = unsafe { metal::EventRef::from_ptr(event.cast()) }.to_owned();
        Self::new(texture, event, value, lease)
    }
    /// Device loss or failed submission permanently invalidates this source.
    pub fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
    }
    /// Whether a failed submission/device-loss owner retired this source.
    pub fn is_retired(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
    }
}
#[cfg(target_os = "macos")]
impl From<std::sync::Arc<NativeTexture>> for SurfaceSource {
    fn from(texture: std::sync::Arc<NativeTexture>) -> Self {
        Self::NativeTexture(texture)
    }
}
