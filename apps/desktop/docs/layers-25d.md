# 2.5D layers

Enable **3D layer** under Properties → Transform, or in the layer context menu.
Text, shapes, images, video, precomps, and extension output stay editable flat
surfaces. Turning 3D off retains the depth channels and their animation.

- Position Z moves toward (negative) or away from (positive) the viewer.
- X/Y Rotation tilts the plane; the existing Rotation channel becomes Z Rotation.
- Orientation X/Y/Z establishes a base pose before the rotation channels.
- Perspective is focal length in millimeters, defaulting to 50 mm, with no upper
  limit. Short lenses exaggerate depth; long lenses flatten it. Values must be
  positive. It supports keyframes and expressions.
  The outermost enabled 3D group supplies one shared perspective to its contents.
- Anchor Z and Scale Z support depth offsets and parent rigs.
- These channels use the existing keyframes, expressions, timeline, save format,
  and Undo transactions. `set_layer` accepts `patch: { threeD: true }`; ordinary
  `set_property` commands address `position.z`, `rotation.x`, `rotation.y`, etc.
- Dragging on the canvas follows perspective. Use the inspector for scale,
  rotation, and anchor editing. Temporarily turn off 3D for path vertex editing.

The composition uses a fixed centered perspective camera (50 mm / 36 mm gate).
At zero depth and rotation, enabling 3D preserves the original 2D appearance.
Enable the switch on a group to transform all its children, including nested
groups and members whose own 3D switch is off. Group scale, orientation,
rotation, depth, keyframes, selection, and canvas dragging share that space.
Moving a member out or ungrouping rebases its pose into editable 3D channels;
animated ungrouping samples composition frames, as with existing 2D groups.

Coplanar artwork keeps its timeline stacking order when rotated, so text and
details stay above their background. Separate 3D surfaces sort by anchor depth; a 2D layer separates depth groups and
keeps its compositing order. Planes remain double-sided. The shared compositor
also renders exports and nested compositions.

This initial version does not include camera layers, lights, shadows, extrusion,
3D transform gizmos, or per-pixel intersections between crossing planes. Effects
continue to operate in composition space. Reparenting preserves the current 3D pose. A hierarchy whose nonuniform
scale and rotations create XZ/YZ shear cannot be decomposed into these channels;
move-out/ungroup rejects that operation atomically and asks for uniform scale.
