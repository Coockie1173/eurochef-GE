"""Transparent geometry of GoldenEye 007 (Wii) levels as objects of its own.

Sidebar (N) > GoldenEye > Transparent. "Select Rooms Without Transparent" selects every
room's meshes but the `_transparent` ones. "Separate Transparent" takes the faces with a
transparent material out of every selected mesh and puts them into a new object next to it,
named after it with `_transparent` behind: `Room01_walls` gives `Room01_walls_transparent`,
which is room 01 as well, and the new object has the same parent, so one below a `RoomXX`
node stays below it.

A material is transparent when it is blended, when its Principled BSDF's Alpha is below 1 or
comes from somewhere, when it has a Transparent BSDF, or when its name has one of the words.

One file: Preferences > Add-ons > Install from Disk, or open it in the Text Editor and Run
Script (that lasts until Blender is closed).
"""

import re

import bmesh
import bpy
from bpy.props import BoolProperty, PointerProperty, StringProperty

bl_info = {
    "name": "GoldenEye Transparent Geometry",
    "description": "Separate the transparent faces of GoldenEye 007 (Wii) level meshes into objects of their own",
    "blender": (4, 2, 0),
    "version": (0, 1, 0),
    "location": "3D View > Sidebar > GoldenEye",
    "category": "Object",
}

SUFFIX = "_transparent"


def words_of(text):
    return [w.strip().lower() for w in text.split(",") if w.strip()]


def is_transparent(material, settings):
    if material is None:
        return False
    name = material.name.lower()
    if any(word in name for word in words_of(settings.words)):
        return True
    # 4.2 on reads every material's old blend mode as hashed: only blended says something
    if getattr(material, "surface_render_method", None) == 'BLENDED' or getattr(material, "blend_method", None) == 'BLEND':
        return True
    if not settings.nodes or material.node_tree is None:
        return False
    for node in material.node_tree.nodes:
        if node.type == 'BSDF_TRANSPARENT' and node.outputs[0].is_linked:
            return True
        if node.type == 'BSDF_PRINCIPLED':
            alpha = node.inputs.get("Alpha")
            if alpha is not None and (alpha.is_linked or alpha.default_value < 0.999):
                return True
    return False


def room_of(obj):
    """The name of what makes the object a room's: itself or a node above it. None for none"""
    above = obj
    while above is not None:
        if above.name.lower().startswith("room"):
            return above.name
        above = above.parent
    return None


def is_separated(obj):
    """Whether "Separate Transparent" made the object: `Room01_transparent`, `..._transparent.001`"""
    return re.sub(r"\.\d+$", "", obj.name).endswith(SUFFIX)


def keep_faces(mesh, keep):
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bm.faces.ensure_lookup_table()
    gone = [f for f in bm.faces if f.index not in keep]
    bmesh.ops.delete(bm, geom=gone, context='FACES')
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()


def separate(obj, settings):
    """Moves the object's transparent faces into a copy of it. Returns the copy, None when
    none of its faces are transparent or all of them are"""
    mesh = obj.data
    slots = {i for i, slot in enumerate(obj.material_slots) if is_transparent(slot.material, settings)}
    transparent = {p.index for p in mesh.polygons if p.material_index in slots}
    if not transparent or len(transparent) == len(mesh.polygons):
        return None

    # the copy has the object's parent and place: below a room's node it is that room's too
    other = obj.copy()
    other.data = mesh.copy()
    other.name = re.sub(r"\.\d+$", "", obj.name) + SUFFIX
    other.data.name = other.name
    for collection in obj.users_collection:
        collection.objects.link(other)
    keep_faces(mesh, set(range(len(mesh.polygons))) - transparent)
    keep_faces(other.data, transparent)
    return other


class Settings(bpy.types.PropertyGroup):
    words: StringProperty(
        name="Words",
        description="A material with one of these in its name is transparent, whatever it looks like. Commas between them",
        default="glass, transparent",
    )
    nodes: BoolProperty(
        name="Look at Nodes",
        description="Transparent as well: a Principled BSDF whose Alpha is below 1 or linked, a Transparent BSDF",
        default=True,
    )


class SeparateTransparent(bpy.types.Operator):
    """Puts the transparent faces of every selected mesh into an object of their own, in the same room"""
    bl_idname = "ge.separate_transparent"
    bl_label = "Separate Transparent"
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        settings = context.scene.ge_transparent
        if context.mode != 'OBJECT':
            self.report({'ERROR'}, "Separating is done in Object Mode")
            return {'CANCELLED'}
        meshes = [o for o in context.selected_objects if o.type == 'MESH']
        if not meshes:
            self.report({'ERROR'}, "Select the meshes to take the transparent faces out of")
            return {'CANCELLED'}

        made, roomless = [], 0
        for obj in meshes:
            # an object that is transparent already isn't gone through again
            if is_separated(obj):
                continue
            if obj.data.users > 1:
                obj.data = obj.data.copy()
            other = separate(obj, settings)
            if other is None:
                continue
            made.append(other)
            if room_of(other) is None:
                roomless += 1

        for obj in made:
            obj.select_set(True)
        text = "{} object(s) of transparent faces".format(len(made))
        if roomless:
            text += ", {} in no room (neither was what they came from)".format(roomless)
        self.report({'INFO'}, text)
        return {'FINISHED'}


class SelectOpaqueRooms(bpy.types.Operator):
    """Selects every room's meshes but the ones "Separate Transparent" made, and nothing else"""
    bl_idname = "ge.select_opaque_rooms"
    bl_label = "Select Rooms Without Transparent"
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        if context.mode != 'OBJECT':
            self.report({'ERROR'}, "Selecting is done in Object Mode")
            return {'CANCELLED'}

        rooms = [o for o in context.view_layer.objects if o.type == 'MESH' and room_of(o) is not None]
        if not rooms:
            self.report({'ERROR'}, "No rooms: nothing is named RoomXX or below something that is")
            return {'CANCELLED'}

        for obj in context.selected_objects:
            obj.select_set(False)
        chosen = [o for o in rooms if not is_separated(o) and o.visible_get()]
        for obj in chosen:
            obj.select_set(True)
        if chosen:
            context.view_layer.objects.active = chosen[0]
        self.report({'INFO'}, "{} room mesh(es), {} transparent left out".format(len(chosen), sum(1 for o in rooms if is_separated(o))))
        return {'FINISHED'}


class TransparentPanel(bpy.types.Panel):
    bl_idname = "VIEW3D_PT_ge_transparent"
    bl_label = "Transparent"
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = "GoldenEye"

    def draw(self, context):
        layout = self.layout
        settings = context.scene.ge_transparent

        column = layout.column(align=True)
        column.prop(settings, "words")
        column.prop(settings, "nodes")

        counted = set()
        for obj in context.selected_objects:
            if obj.type == 'MESH':
                counted.update(s.material.name for s in obj.material_slots if is_transparent(s.material, settings))
        layout.label(text="Selected: {} transparent material(s)".format(len(counted)))
        layout.operator(SeparateTransparent.bl_idname, icon='MOD_EXPLODE')
        layout.operator(SelectOpaqueRooms.bl_idname, icon='RESTRICT_SELECT_OFF')


CLASSES = (Settings, SeparateTransparent, SelectOpaqueRooms, TransparentPanel)


def register():
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.Scene.ge_transparent = PointerProperty(type=Settings)


def unregister():
    del bpy.types.Scene.ge_transparent
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()
