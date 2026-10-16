//! What one render produces (contract 2's `FrameRequest`, as the render
//! graph sees it).

use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_protocol::frame::{FrameRequest, MAX_OUTPUT_DIMENSION, MAX_SCALE, MIN_SCALE};
use pm_protocol::validate::Validate;

use crate::error::RenderError;

/// A render of one composition at one time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    /// The full output, in output pixels. Composition pixel `(x, y)` lands at
    /// output pixel `(x · sx, y · sy)`.
    pub output_size: Size,
    /// Output pixels per composition pixel, per axis. `None` derives it the
    /// way the legacy compositor does: `W / max(1, comp.w || W)` and
    /// `H / max(1, comp.h || H)` from the document's composition size.
    pub scale: Option<(f64, f64)>,
    /// The part of the output to render (region of interest, section 8:
    /// final output only); `None` renders all of it. The returned texture is
    /// the region's size.
    pub region: Option<PixelRect>,
    /// Intermediate storage: `rgba16float` or `rgba32float`.
    pub precision: Precision,
    /// Also render the depth and object-id surface (contract 2).
    pub depth_id: bool,
    /// Draw the composition background. Off for today's alpha exports and
    /// transparent captures (`renderToPixels(…, {transparent: true})`).
    pub background: bool,
    /// The frame feeds an export: media decodes at export depth
    /// (amendment 6: ProRes 422 as `v216`).
    pub export: bool,
}

impl RenderRequest {
    /// The whole composition at `scale` (output size rounded to whole
    /// pixels, at least 1).
    pub fn at_scale(composition: Size, scale: f64, precision: Precision) -> RenderRequest {
        let px = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
        RenderRequest {
            output_size: Size::new(px(composition.width), px(composition.height)),
            scale: Some((scale, scale)),
            region: None,
            precision,
            depth_id: false,
            background: true,
            export: false,
        }
    }

    /// An output of a given size; the scale is the size ratio per axis, as
    /// the legacy compositor derives it (`W / comp.w`, `H / comp.h`).
    pub fn sized(output: Size, precision: Precision) -> RenderRequest {
        RenderRequest {
            output_size: output,
            scale: None,
            region: None,
            precision,
            depth_id: false,
            background: true,
            export: false,
        }
    }

    /// Contract 2's request. A request carrying a view override is refused
    /// here, so a caller that does not apply the view routes it; callers
    /// that do use [`RenderRequest::from_frame_request_with_view`] and
    /// `Renderer::set_view`.
    pub fn from_frame_request(request: &FrameRequest) -> Result<RenderRequest, RenderError> {
        request
            .validate()
            .map_err(|v| RenderError::Request(v.to_string()))?;
        if request.view.is_some() {
            return Err(RenderError::Unsupported(vec![
                crate::capability::Unsupported::request(
                    "view override (editor camera, shading, Local View)",
                ),
            ]));
        }
        Ok(RenderRequest {
            output_size: request.output_size,
            scale: Some((request.scale, request.scale)),
            region: request.region,
            precision: request.precision,
            depth_id: request.depth_id,
            background: true,
            export: request.purpose == pm_protocol::frame::FramePurpose::Export,
        })
    }

    /// Contract 2's request with its view override, for callers that pass
    /// the view to `Renderer::set_view`: the override is dropped for exports
    /// and agent captures (`FramePurpose::is_final`), as section 18 requires.
    pub fn from_frame_request_with_view(
        request: &FrameRequest,
    ) -> Result<(RenderRequest, Option<pm_protocol::frame::ViewOverride>), RenderError> {
        let view = request.view.clone().filter(|_| !request.purpose.is_final());
        let mut without = request.clone();
        without.view = None;
        Ok((RenderRequest::from_frame_request(&without)?, view))
    }

    /// The size of the texture a render returns.
    pub fn target_size(&self) -> Size {
        match self.region {
            Some(r) => Size::new(r.width, r.height),
            None => self.output_size,
        }
    }

    /// The region's origin in output pixels.
    pub fn origin(&self) -> (u32, u32) {
        self.region.map_or((0, 0), |r| (r.x, r.y))
    }

    pub fn validate(&self) -> Result<(), RenderError> {
        let s = self.output_size;
        if s.is_empty() || s.width > MAX_OUTPUT_DIMENSION || s.height > MAX_OUTPUT_DIMENSION {
            return Err(RenderError::Request(format!(
                "output size {s:?} outside 1..{MAX_OUTPUT_DIMENSION}"
            )));
        }
        if let Some((sx, sy)) = self.scale {
            for v in [sx, sy] {
                if !(v.is_finite() && (MIN_SCALE * 0.999..=MAX_SCALE * 1.001).contains(&v)) {
                    return Err(RenderError::Request(format!("scale {v} outside 0.01..16")));
                }
            }
        }
        if let Some(r) = self.region
            && (r.is_empty() || !s.contains(&r))
        {
            return Err(RenderError::Request(format!(
                "region {r:?} is empty or outside {s:?}"
            )));
        }
        Ok(())
    }
}
