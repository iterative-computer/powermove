# Corner Pin Snap

Register this extension and apply **Corner Pin Snap** to a text or image layer.
The effect samples the original pixels through an inverse homography; it does
not rotate a flat rectangle in 3D. Preview and export sample at render resolution.

Start with heavy white **BREAK** and **FREE** text layers on a red background.
Use Amount 60, Every N frames 12, Perspective 70 and Snap sharpness 100. Give
FREE a different Layer seed offset so each line snaps independently. Hold freezes
the first seeded pose. Reduce Snap sharpness for up to two frames of easing.

TL, TR, BR and BL move the four corners, in percent of the layer's bounds.
Every corner axis is a normal keyframe channel. Amount 0 turns off random
motion and leaves the manual pins active. Seed and Layer seed offset choose a
repeatable pose stream. Buffer padding declares the required sampling reach in
composition pixels; increase it for large layers/offsets, within the memory
budget. Folded or singular manual quadrilaterals should be avoided; a singular
homography bypasses the warp for that frame.

`u_frame` is supplied by the native compositor using the owning composition's
frame rate, so seeking and non-30-fps exports use the same poses. Raw shader
uniforms are declared in the source. The reference uses the existing effect
catalog and compositor; no built-in recognizes its id.
