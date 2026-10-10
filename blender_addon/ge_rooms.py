"""Rooms for GoldenEye 007 (Wii) levels out of geometry that has none: an outside, a terrain.

Sidebar (N) > GoldenEye > Rooms. "Cut Into Rooms" cuts the selected meshes along the faces of
boxes, makes a `RoomNN` of what lies in each box and a `Portal_NN_MM` quad wherever two boxes
with something in them touch: the names `eurochef-cli ge new-map` reads rooms and portals by.

The boxes are the objects named `ref_roombox...` (the importer leaves out what is named
`ref_...`). "Make Room Boxes" puts a grid of them over the selected meshes, to be moved, sized,
added to and deleted before the cut; without any in the scene the cut uses that grid as it is.
A box counts as the box around it along the axes: one that is turned is larger than it looks.
What lies in no box stays in its object, and the importer gives it the room it lies in.

One file: Preferences > Add-ons > Install from Disk, or open it in the Text Editor and Run
Script (that lasts until Blender is closed).
"""

import math
import re

import bmesh
import bpy
from bpy.props import BoolProperty, FloatProperty, IntProperty, PointerProperty
from mathutils import Matrix, Vector

bl_info = {
    "name": "GoldenEye Rooms",
    "description": "Cut geometry into rooms with portals between them for GoldenEye 007 (Wii) levels",
    "blender": (4, 2, 0),
    "version": (0, 1, 0),
    "location": "3D View > Sidebar > GoldenEye",
    "category": "Object",
}

BOX_PREFIX = "ref_roombox"
ROOM_LAYER = "ge_room"
# what isn't a room's geometry: left where it is
HELPERS = ("portal", "collision", "col_", "ucx_", "vault", "climb", "ladder", "reference", "ref_", "lightmap", "sky", "spawn", "mp_spawn", "player_start")
# the game keeps a zone's number in a byte
MAX_ROOMS = 255
# how far from a plane a vertex is that counts as on it
ON_PLANE = 1e-4
# how far apart two boxes' faces are that still touch
TOUCHING = 0.01
# the least width of a portal
LEAST_PORTAL = 0.05


def is_named(obj, prefixes):
    names = [obj.name.lower()]
    if obj.data is not None:
        names.append(obj.data.name.lower())
    return any(name.startswith(prefixes) for name in names)


def is_box(obj):
    return obj.name.lower().startswith(BOX_PREFIX)


def world_box(obj):
    """The box around an object along the world's axes: its two corners"""
    corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
    return (
        tuple(min(c[k] for c in corners) for k in range(3)),
        tuple(max(c[k] for c in corners) for k in range(3)),
    )


def grid_boxes(low, high, cell, floors, below, above):
    """Boxes of about `cell` by `cell` that fill the bounds, `floors` of them on top of each
    other. They are all the same size, so the last isn't a sliver"""
    low = (low[0], low[1], low[2] - below)
    high = (high[0], high[1], high[2] + above)
    counts = [max(1, math.ceil((high[k] - low[k]) / cell - 1e-6)) for k in range(2)] + [max(1, floors)]
    steps = [(high[k] - low[k]) / counts[k] for k in range(3)]
    boxes = []
    for z in range(counts[2]):
        for y in range(counts[1]):
            for x in range(counts[0]):
                at = (x, y, z)
                boxes.append((
                    tuple(low[k] + at[k] * steps[k] for k in range(3)),
                    tuple(low[k] + (at[k] + 1) * steps[k] for k in range(3)),
                ))
    return boxes


def portal_between(a, b):
    """The quad two boxes meet in, None when they don't. Boxes that reach into each other meet
    in the middle of what they share, across its thinnest side"""
    low = [max(a[0][k], b[0][k]) for k in range(3)]
    high = [min(a[1][k], b[1][k]) for k in range(3)]
    sizes = [high[k] - low[k] for k in range(3)]
    if min(sizes) < -TOUCHING:
        return None
    axis = sizes.index(min(sizes))
    u, v = [k for k in range(3) if k != axis]
    if sizes[u] < LEAST_PORTAL or sizes[v] < LEAST_PORTAL:
        return None
    middle = (low[axis] + high[axis]) / 2
    corners = []
    for cu, cv in ((low[u], low[v]), (high[u], low[v]), (high[u], high[v]), (low[u], high[v])):
        corner = [0.0, 0.0, 0.0]
        corner[axis], corner[u], corner[v] = middle, cu, cv
        corners.append(tuple(corner))
    return corners


def box_of(point, boxes):
    """The index of the first box a point is in, -1 for none"""
    for index, (low, high) in enumerate(boxes):
        if all(low[k] - ON_PLANE <= point[k] <= high[k] + ON_PLANE for k in range(3)):
            return index
    return -1


def cut_planes(boxes):
    """Every plane a box has a face in, as (axis, where) with the boxes whose face it is"""
    planes = {}
    for box in boxes:
        for axis in range(3):
            for side in box:
                planes.setdefault((axis, round(side[axis], 4)), []).append(box)
    return planes


def cut(bm, axis, where, boxes):
    """Cuts the faces that cross a plane, those next to one of the boxes it belongs to only: a
    box's face doesn't cut the whole level"""
    others = [k for k in range(3) if k != axis]
    crossing = []
    for face in bm.faces:
        along = [v.co[axis] for v in face.verts]
        if min(along) >= where - ON_PLANE or max(along) <= where + ON_PLANE:
            continue
        spans = [(min(v.co[k] for v in face.verts), max(v.co[k] for v in face.verts)) for k in others]
        if any(all(spans[i][1] >= low[k] and spans[i][0] <= high[k] for i, k in enumerate(others)) for low, high in boxes):
            crossing.append(face)
    if not crossing:
        return
    geom = set(crossing)
    for face in crossing:
        geom.update(face.edges)
        geom.update(face.verts)
    point, normal = Vector(), Vector()
    point[axis], normal[axis] = where, 1.0
    bmesh.ops.bisect_plane(bm, geom=list(geom), dist=ON_PLANE, plane_co=point, plane_no=normal)


def cut_object(obj, boxes):
    """Cuts an object's mesh along the boxes' faces. Returns a mesh for each box that has some
    of it, in the world's space, and the cut mesh with every face's box: the object itself is
    as it was until `keep_rest`"""
    to_world = obj.matrix_world.copy()
    mirrored = to_world.determinant() < 0
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bm.transform(to_world)
    if mirrored:
        bmesh.ops.reverse_faces(bm, faces=bm.faces[:])

    for (axis, where), owners in sorted(cut_planes(boxes).items()):
        cut(bm, axis, where, owners)

    layer = bm.faces.layers.int.new(ROOM_LAYER)
    found = set()
    for face in bm.faces:
        face[layer] = box_of(face.calc_center_median(), boxes) + 1
        found.add(face[layer])

    meshes = {}
    for number in sorted(found - {0}):
        part = bm.copy()
        part_layer = part.faces.layers.int[ROOM_LAYER]
        bmesh.ops.delete(part, geom=[f for f in part.faces if f[part_layer] != number], context='FACES')
        part.faces.layers.int.remove(part_layer)
        part.normal_update()
        mesh = obj.data.copy()
        part.to_mesh(mesh)
        part.free()
        meshes[number - 1] = mesh

    return meshes, bm


def keep_rest(obj, bm):
    """Leaves an object what lay in no box. Returns whether that is anything"""
    layer = bm.faces.layers.int[ROOM_LAYER]
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if f[layer] != 0], context='FACES')
    bm.faces.layers.int.remove(layer)
    left = len(bm.faces) > 0
    if obj.matrix_world.determinant() < 0:
        bmesh.ops.reverse_faces(bm, faces=bm.faces[:])
    bm.transform(obj.matrix_world.inverted())
    bm.normal_update()
    bm.to_mesh(obj.data)
    obj.data.update()
    return left


def first_free_number(scene):
    """Rooms are numbered on from the ones the scene has"""
    highest = 0
    for obj in scene.objects:
        match = re.match(r"room[_ -]*(\d+)", obj.name.lower())
        if match:
            highest = max(highest, int(match.group(1)))
    return highest + 1


def add_empty(scene, name, at):
    empty = bpy.data.objects.new(name, None)
    empty.empty_display_size = 1.0
    empty.location = at
    scene.collection.objects.link(empty)
    return empty


def add_box(scene, name, box):
    """A cube drawn as wires, a unit cube scaled: sizing it in Object Mode sizes the room"""
    low, high = box
    corners = [(x, y, z) for x in (-0.5, 0.5) for y in (-0.5, 0.5) for z in (-0.5, 0.5)]
    faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(corners, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    obj.location = [(low[k] + high[k]) / 2 for k in range(3)]
    obj.scale = [high[k] - low[k] for k in range(3)]
    obj.display_type = 'WIRE'
    obj.hide_render = True
    scene.collection.objects.link(obj)
    return obj


def add_portal(scene, name, corners):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(corners, [], [(0, 1, 2, 3)])
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    obj.display_type = 'WIRE'
    obj.hide_render = True
    scene.collection.objects.link(obj)
    return obj


def geometry_of(context):
    return [o for o in context.selected_objects if o.type == 'MESH' and not is_box(o) and not is_named(o, HELPERS)]


def bounds_of(objects):
    boxes = [world_box(o) for o in objects]
    return (
        tuple(min(b[0][k] for b in boxes) for k in range(3)),
        tuple(max(b[1][k] for b in boxes) for k in range(3)),
    )


class Settings(bpy.types.PropertyGroup):
    cell: FloatProperty(
        name="Room Size",
        description="How wide and deep a room of the grid is, about: they are all made the same size",
        default=40.0, min=1.0, soft_max=200.0, unit='LENGTH',
    )
    floors: IntProperty(name="Floors", description="How many rooms of the grid are on top of each other", default=1, min=1, max=16)
    above: FloatProperty(
        name="Above",
        description="How far over the geometry the grid's rooms (and so the portals) reach: a camera higher up is in no room",
        default=20.0, min=0.0, unit='LENGTH',
    )
    below: FloatProperty(name="Below", description="How far under the geometry the grid's rooms reach", default=2.0, min=0.0, unit='LENGTH')
    portals: BoolProperty(name="Portals", description="Make a portal wherever two rooms touch", default=True)
    keep_boxes: BoolProperty(name="Keep Boxes", description="Leave the boxes in the scene after the cut", default=False)


class MakeRoomBoxes(bpy.types.Operator):
    """Puts a grid of boxes over the selected meshes: move, size, add and delete them, then cut"""
    bl_idname = "ge.make_room_boxes"
    bl_label = "Make Room Boxes"
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        settings = context.scene.ge_rooms
        geometry = geometry_of(context)
        if not geometry:
            self.report({'ERROR'}, "Select the meshes to cut into rooms")
            return {'CANCELLED'}
        low, high = bounds_of(geometry)
        boxes = grid_boxes(low, high, settings.cell, settings.floors, settings.below, settings.above)
        for number, box in enumerate(boxes, 1):
            add_box(context.scene, "{}_{:03d}".format(BOX_PREFIX, number), box)
        self.report({'INFO'}, "{} box(es)".format(len(boxes)))
        return {'FINISHED'}


class CutIntoRooms(bpy.types.Operator):
    """Cuts the selected meshes along the boxes' faces into rooms, with portals where boxes touch"""
    bl_idname = "ge.cut_into_rooms"
    bl_label = "Cut Into Rooms"
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        scene = context.scene
        settings = scene.ge_rooms
        if context.mode != 'OBJECT':
            self.report({'ERROR'}, "Cutting is done in Object Mode")
            return {'CANCELLED'}
        geometry = geometry_of(context)
        if not geometry:
            self.report({'ERROR'}, "Select the meshes to cut into rooms")
            return {'CANCELLED'}

        box_objects = sorted((o for o in scene.objects if is_box(o)), key=lambda o: o.name)
        if box_objects:
            boxes = [world_box(o) for o in box_objects]
        else:
            low, high = bounds_of(geometry)
            boxes = grid_boxes(low, high, settings.cell, settings.floors, settings.below, settings.above)

        # a mesh two objects share is cut once for each otherwise
        for obj in geometry:
            if obj.data.users > 1:
                obj.data = obj.data.copy()

        parts, cuts = {}, []
        for obj in geometry:
            meshes, bm = cut_object(obj, boxes)
            for index, mesh in meshes.items():
                parts.setdefault(index, []).append((obj, mesh))
            cuts.append((obj, bm, bool(meshes)))

        problem = None
        if len(parts) > MAX_ROOMS:
            problem = "{} rooms, the game has {} at most: larger boxes".format(len(parts), MAX_ROOMS)
        elif not parts:
            problem = "Nothing of the selection lies in a box"
        if problem is not None:
            for _, bm, _ in cuts:
                bm.free()
            for pieces in parts.values():
                for _, mesh in pieces:
                    bpy.data.meshes.remove(mesh)
            self.report({'ERROR'}, problem)
            return {'CANCELLED'}

        first = first_free_number(scene)
        numbers = {index: first + order for order, index in enumerate(sorted(parts))}
        digits = max(2, len(str(max(numbers.values()))))
        room_name = lambda index: "Room{:0{}d}".format(numbers[index], digits)

        made = []
        for index, pieces in sorted(parts.items()):
            low, high = boxes[index]
            room = add_empty(scene, room_name(index), [(low[k] + high[k]) / 2 for k in range(3)])
            for source, mesh in pieces:
                piece = source.copy()
                piece.data = mesh
                piece.name = "{}_{}".format(room.name, re.sub(r"\.\d+$", "", source.name))
                mesh.name = piece.name
                for collection in source.users_collection:
                    collection.objects.link(piece)
                # the mesh is in the world's space: the piece stands where the world does, so
                # the room's place is taken off again. Not by way of matrix_world: the new
                # empty's isn't worked out yet, and the piece would end up moved by its place
                piece.parent = room
                piece.matrix_parent_inverse = Matrix.Translation(room.location).inverted()
                piece.matrix_basis = Matrix.Identity(4)
                made.append(piece)

        portals = 0
        if settings.portals:
            indices = sorted(parts)
            for i, a in enumerate(indices):
                for b in indices[i + 1:]:
                    corners = portal_between(boxes[a], boxes[b])
                    if corners is not None:
                        add_portal(scene, "Portal_{}_{}".format(room_name(a)[4:], room_name(b)[4:]), corners)
                        portals += 1

        # what was cut up whole is gone, what wasn't keeps the rest
        for obj, bm, was_cut in cuts:
            left = keep_rest(obj, bm) if was_cut else True
            bm.free()
            if not left and not obj.children:
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
        if not settings.keep_boxes:
            for obj in box_objects:
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh is not None and mesh.users == 0:
                    bpy.data.meshes.remove(mesh)

        for obj in made:
            obj.select_set(True)
        context.view_layer.objects.active = made[0]
        self.report({'INFO'}, "{} room(s), {} portal(s)".format(len(parts), portals))
        return {'FINISHED'}


class RoomsPanel(bpy.types.Panel):
    bl_idname = "VIEW3D_PT_ge_rooms"
    bl_label = "Rooms"
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = "GoldenEye"

    def draw(self, context):
        layout = self.layout
        settings = context.scene.ge_rooms
        boxes = sum(1 for o in context.scene.objects if is_box(o))

        column = layout.column(align=True)
        column.prop(settings, "cell")
        column.prop(settings, "floors")
        column.prop(settings, "above")
        column.prop(settings, "below")
        layout.operator(MakeRoomBoxes.bl_idname, icon='MESH_GRID')

        layout.separator()
        layout.label(text="Scene: {} box(es)".format(boxes) if boxes else "No boxes: the grid is used")
        column = layout.column(align=True)
        column.prop(settings, "portals")
        column.prop(settings, "keep_boxes")
        layout.operator(CutIntoRooms.bl_idname, icon='MOD_BOOLEAN')


CLASSES = (Settings, MakeRoomBoxes, CutIntoRooms, RoomsPanel)


def register():
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.Scene.ge_rooms = PointerProperty(type=Settings)


def unregister():
    del bpy.types.Scene.ge_rooms
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()
