// RGBA16F entries of the 2D draws: the nearest half of each value
// (Metal's render-target conversion truncates). Blended draws still round
// their blend result in hardware.
@fragment fn fs_draw_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_draw_value(in)); }
@fragment fn fs_solid_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_solid_value(in)); }
@fragment fn fs_background_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_background_value(in)); }
