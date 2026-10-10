"""Lightmaps for GoldenEye 007 (Wii) levels: bake them and get them into the .glb.

Sidebar (N) > GoldenEye. "Bake" gives every selected mesh a second UV map named `lightmap`,
bakes the scene's light into an image of its own and remembers it on the object. "Export"
writes a .glb in which each of those objects comes twice: as it is, and as `lightmap_NAME`,
the same triangles with the baked image as their one texture and the lightmap UV map as their
one UV map. The copies exist only while the file is written.

A lamp (a material with emission) lights the bake and gets no lightmap itself: the game can only
darken by one, and a lamp gets next to no light of its own. Its faces aren't in the copy.

One file: Preferences > Add-ons > Install from Disk, or open it in the Text Editor and Run
Script (that lasts until Blender is closed).
"""

import math
import os
import re

import bmesh
import bpy
from bpy.props import BoolProperty, EnumProperty, FloatProperty, IntProperty, PointerProperty, StringProperty
from bpy_extras.io_utils import ExportHelper

bl_info = {
    "name": "GoldenEye Lightmaps",
    "description": "Bake lightmaps for GoldenEye 007 (Wii) levels and export them with the scene",
    "blender": (4, 2, 0),
    "version": (0, 1, 0),
    "location": "3D View > Sidebar > GoldenEye",
    "category": "Import-Export",
}

UV_NAME = "lightmap"
COPY_PREFIX = "lightmap_"
BAKE_NODE = "GE Lightmap Bake"
# what eurochef doesn't draw: a portal is a quad in a doorway, a collision mesh a box around a
# crate. Left in the bake they would be walls, and no light would get from one room into the next
HELPERS = ("portal", "collision", "col_", "ucx_", "vault", "climb", "ladder", "reference", "ref_", "lightmap")
SKY = "sky"


def is_named(obj, prefixes):
    names = [obj.name.lower()]
    if obj.data is not None:
        names.append(obj.data.name.lower())
    return any(name.startswith(prefixes) for name in names)


def hide_helpers(scene, sky_too):
    """Takes what isn't a part of the picture out of the render, parents' children with them.
    Returns the objects to show again"""
    hidden = []
    for obj in scene.objects:
        if obj.hide_render:
            continue
        above = obj
        while above is not None:
            if is_named(above, HELPERS) or (sky_too and is_named(above, (SKY,))):
                obj.hide_render = True
                hidden.append(obj)
                break
            above = above.parent
    return hidden


class Settings(bpy.types.PropertyGroup):
    size: EnumProperty(
        name="Size",
        description="Of each object's lightmap. The game's own are 128",
        items=[(s, s, "") for s in ("64", "128", "256", "512")],
        default="128",
    )
    samples: IntProperty(name="Samples", description="Cycles samples for the bake", default=64, min=1, max=4096)
    margin: IntProperty(name="Margin", description="Pixels the bake bleeds over an island's edge", default=4, min=0, max=32)
    unwrap: EnumProperty(
        name="Unwrap",
        description="How a new lightmap UV map is made",
        items=[
            ('SMART', "Smart UV Project", "Faces that lie flat together stay together: few seams, some of the image unused"),
            ('PACK', "Lightmap Pack", "Every face its own rectangle: the whole image used, a seam at every edge"),
        ],
        default='SMART',
    )
    island_margin: FloatProperty(
        name="Island Margin", description="Room between the islands of a new lightmap UV map", default=0.01, min=0.0, max=0.5
    )
    exposure: FloatProperty(
        name="Exposure",
        description="The baked light times this. The game can only darken by a lightmap: what is white leaves the level as it is",
        default=1.0, min=0.1, max=16.0,
    )
    split: BoolProperty(
        name="Split Large Meshes",
        description="Cut a mesh that is too large for one lightmap into several objects, each with a lightmap of its own. "
        "The pieces are named NAME_lm2, NAME_lm3...: a room's pieces stay that room",
        default=False,
    )
    density: FloatProperty(
        name="Texels per Metre",
        description="The least a lightmap is to have of a split mesh: more of them, more and smaller pieces",
        default=8.0, min=0.5, max=256.0,
    )
    denoise: BoolProperty(
        name="Denoise",
        description="Smooth the bake's grain. Grain is different on every face, so it shows where two faces meet",
        default=True,
    )
    hide_sky: BoolProperty(
        name="Bake Without the Sky",
        description="Leave the sky's mesh out of the bake: around the whole level, it keeps the sun and the world's light out",
        default=True,
    )
    unwrap_again: BoolProperty(
        name="Unwrap Again", description="Make the lightmap UV map anew for objects that have one already", default=False
    )


def has_lightmap(obj):
    return obj.type == 'MESH' and obj.ge_lightmap is not None and UV_NAME in obj.data.uv_layers


def unwrap(context, obj, settings):
    """A UV map of the object's own for the lightmap: every triangle its own place"""
    mesh = obj.data
    layer = mesh.uv_layers.get(UV_NAME) or mesh.uv_layers.new(name=UV_NAME)
    mesh.uv_layers.active = layer

    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.reveal()
    bpy.ops.mesh.select_all(action='SELECT')
    if settings.unwrap == 'PACK':
        bpy.ops.uv.lightmap_pack(
            PREF_CONTEXT='ALL_FACES',
            PREF_PACK_IN_ONE=False,
            PREF_NEW_UVLAYER=False,
            PREF_BOX_DIV=12,
            PREF_MARGIN_DIV=min(1.0, max(0.1, settings.island_margin * 30.0)),
        )
    else:
        # stretched to the image's sides: a long room would only use a band of it
        bpy.ops.uv.smart_project(
            angle_limit=math.radians(66.0), island_margin=settings.island_margin, scale_to_bounds=True
        )
    bpy.ops.object.mode_set(mode='OBJECT')


def expose(image, factor):
    """The image's light times a factor, white at the most"""
    pixels = [0.0] * (image.size[0] * image.size[1] * image.channels)
    image.pixels.foreach_get(pixels)
    channels = image.channels
    for i in range(len(pixels)):
        if i % channels < 3:
            pixels[i] = min(pixels[i] * factor, 1.0)
    image.pixels.foreach_set(pixels)
    image.update()


# how much of a lightmap its triangles get to use, the rest is the room between the islands
FILL = 0.6
MAX_PIECES = 32


def world_area(obj):
    scale = obj.matrix_world.to_scale()
    return sum(p.area for p in obj.data.polygons) * abs(scale.x * scale.y * scale.z) ** (2.0 / 3.0)


def pieces_wanted(obj, size, density):
    """How many lightmaps of this size the mesh needs to get its texels per metre"""
    holds = size * size * FILL / (density * density)
    return max(1, min(MAX_PIECES, math.ceil(world_area(obj) / holds - 1e-6)))


def piece_name(name, number):
    return "{}_lm{}".format(re.sub(r"\.\d+$", "", name), number)


def keep_faces(mesh, keep):
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bm.faces.ensure_lookup_table()
    gone = [f for f in bm.faces if f.index not in keep]
    bmesh.ops.delete(bm, geom=gone, context='FACES')
    bm.to_mesh(mesh)
    bm.free()
    mesh.update()


def halve(obj, share):
    """Cuts the mesh in two across its longest side, `share` of its area staying in it. Returns
    the object the rest went to, None when there is nothing to cut"""
    mesh = obj.data
    if len(mesh.polygons) < 2:
        return None
    centres = [obj.matrix_world @ p.center for p in mesh.polygons]
    spans = [max(c[k] for c in centres) - min(c[k] for c in centres) for k in range(3)]
    axis = spans.index(max(spans))
    order = sorted(range(len(centres)), key=lambda i: centres[i][axis])
    total = sum(p.area for p in mesh.polygons)
    stays, area = set(), 0.0
    for i in order[:-1]:
        stays.add(i)
        area += mesh.polygons[i].area
        if area >= total * share:
            break

    other = obj.copy()
    other.data = mesh.copy()
    for collection in obj.users_collection:
        collection.objects.link(other)
    keep_faces(mesh, stays)
    keep_faces(other.data, set(range(len(centres))) - stays)
    return other


def split(obj, pieces):
    """The mesh as this many objects of about the same area, each a box of its own"""
    if pieces <= 1:
        return [obj]
    first = pieces // 2
    other = halve(obj, first / pieces)
    if other is None:
        return [obj]
    return split(obj, first) + split(other, pieces - first)


def split_for_lightmaps(obj, size, density):
    """Cuts the object up when one lightmap isn't enough for it. Its lightmap, if it had one, is
    of the whole and goes; the pieces are unwrapped anew"""
    pieces = pieces_wanted(obj, size, density)
    if pieces <= 1:
        return [obj]
    name = obj.name
    parts = split(obj, pieces)
    for number, part in enumerate(parts, 1):
        if part is not obj:
            part.name = piece_name(name, number)
            part.data.name = part.name
        part.ge_lightmap = None
        layer = part.data.uv_layers.get(UV_NAME)
        if layer is not None:
            part.data.uv_layers.remove(layer)
    return parts


def is_black(image):
    """Nothing was baked into it, or no light reaches the object"""
    pixels = [0.0] * (image.size[0] * image.size[1] * image.channels)
    image.pixels.foreach_get(pixels)
    channels = image.channels
    return not any(pixels[i] > 0.004 for i in range(len(pixels)) if i % channels < 3)


def add_bake_nodes(obj, image, placeholder):
    """Cycles bakes into the active image node of each material. Returns what to undo"""
    added = []
    filled = []
    mesh = obj.data
    if len(mesh.materials) == 0:
        mesh.materials.append(placeholder)
        filled.append(None)
    for i, material in enumerate(mesh.materials):
        if material is None:
            mesh.materials[i] = placeholder
            filled.append(i)
            material = placeholder
        if material.node_tree is None:
            material.use_nodes = True
        nodes = material.node_tree.nodes
        if BAKE_NODE in nodes:
            node = nodes[BAKE_NODE]
        else:
            node = nodes.new('ShaderNodeTexImage')
            node.name = BAKE_NODE
            added.append((material, nodes.active))
        node.image = image
        nodes.active = node
    return added, filled


def remove_bake_nodes(obj, added, filled):
    for material, was_active in added:
        nodes = material.node_tree.nodes
        if BAKE_NODE in nodes:
            nodes.remove(nodes[BAKE_NODE])
        if was_active is not None:
            nodes.active = was_active
    mesh = obj.data
    for i in filled:
        if i is None:
            mesh.materials.clear()
        else:
            mesh.materials[i] = None


def image_for(obj, size):
    image = obj.ge_lightmap
    if image is None:
        image = bpy.data.images.new("Lightmap_" + obj.name, width=size, height=size, alpha=False)
    elif image.size[0] != size or image.size[1] != size:
        image.scale(size, size)
    return image


class BakeLightmaps(bpy.types.Operator):
    """Bake the scene's light into a lightmap for each selected mesh"""
    bl_idname = "ge.bake_lightmaps"
    bl_label = "Bake Lightmaps"
    bl_options = {'REGISTER', 'UNDO'}

    @classmethod
    def poll(cls, context):
        return context.mode == 'OBJECT' and any(o.type == 'MESH' for o in context.selected_objects)

    def execute(self, context):
        settings = context.scene.ge_lightmaps
        scene = context.scene
        targets = [o for o in context.selected_objects if o.type == 'MESH' and len(o.data.polygons) > 0]
        if not targets:
            self.report({'ERROR'}, "No mesh selected")
            return {'CANCELLED'}

        was_selected = list(context.selected_objects)
        was_active = context.view_layer.objects.active
        was_engine = scene.render.engine
        scene.render.engine = 'CYCLES'
        was_samples = scene.cycles.samples
        scene.cycles.samples = settings.samples
        was_denoising = scene.cycles.use_denoising
        scene.cycles.use_denoising = settings.denoise
        placeholder = bpy.data.materials.new("GE Lightmap Placeholder")
        if placeholder.node_tree is None:
            placeholder.use_nodes = True

        hidden = hide_helpers(scene, settings.hide_sky)

        size = int(settings.size)
        if settings.split:
            whole = targets
            targets = []
            for obj in whole:
                targets.extend(split_for_lightmaps(obj, size, settings.density))
            was_selected = was_selected + [o for o in targets if o not in was_selected]
            if len(targets) > len(whole):
                self.report({'INFO'}, "{} mesh(es) cut into {}".format(len(whole), len(targets)))
        baked = 0
        failed = []
        context.window_manager.progress_begin(0, len(targets))
        try:
            for n, obj in enumerate(targets):
                bpy.ops.object.select_all(action='DESELECT')
                obj.select_set(True)
                context.view_layer.objects.active = obj
                mesh = obj.data
                was_uv = mesh.uv_layers.active_index

                if UV_NAME not in mesh.uv_layers or settings.unwrap_again:
                    unwrap(context, obj, settings)
                mesh.uv_layers.active = mesh.uv_layers[UV_NAME]

                image = image_for(obj, size)
                added, filled = add_bake_nodes(obj, image, placeholder)
                try:
                    # the light alone: the texture stays what the level draws under it. everything
                    # is said here: what isn't comes from the scene's own bake settings, and a
                    # scene that last baked to vertex colours would leave the image black
                    bpy.ops.object.bake(
                        type='DIFFUSE',
                        pass_filter={'DIRECT', 'INDIRECT'},
                        target='IMAGE_TEXTURES',
                        save_mode='INTERNAL',
                        use_selected_to_active=False,
                        use_split_materials=False,
                        margin=settings.margin,
                        use_clear=True,
                        uv_layer=UV_NAME,
                    )
                    if abs(settings.exposure - 1.0) > 1e-3:
                        expose(image, settings.exposure)
                    image.pack()
                    obj.ge_lightmap = image
                    baked += 1
                    if is_black(image):
                        failed.append("{}: baked black, no light reaches it".format(obj.name))
                except RuntimeError as error:
                    failed.append("{}: {}".format(obj.name, str(error).strip()))
                finally:
                    remove_bake_nodes(obj, added, filled)
                    if 0 <= was_uv < len(mesh.uv_layers):
                        mesh.uv_layers.active_index = was_uv
                context.window_manager.progress_update(n + 1)
        finally:
            context.window_manager.progress_end()
            for obj in hidden:
                obj.hide_render = False
            bpy.data.materials.remove(placeholder)
            scene.cycles.samples = was_samples
            scene.cycles.use_denoising = was_denoising
            scene.render.engine = was_engine
            bpy.ops.object.select_all(action='DESELECT')
            for obj in was_selected:
                obj.select_set(True)
            context.view_layer.objects.active = was_active

        for line in failed:
            self.report({'WARNING'}, line)
        self.report({'INFO'}, "Baked {} lightmap(s), {} warning(s)".format(baked, len(failed)))
        return {'FINISHED'} if baked else {'CANCELLED'}


class RemoveLightmaps(bpy.types.Operator):
    """Take the lightmap off the selected meshes: the image and the UV map"""
    bl_idname = "ge.remove_lightmaps"
    bl_label = "Remove Lightmaps"
    bl_options = {'REGISTER', 'UNDO'}

    @classmethod
    def poll(cls, context):
        return context.mode == 'OBJECT' and any(o.type == 'MESH' and o.ge_lightmap for o in context.selected_objects)

    def execute(self, context):
        for obj in context.selected_objects:
            if obj.type != 'MESH':
                continue
            image = obj.ge_lightmap
            obj.ge_lightmap = None
            if image is not None and image.users == 0:
                bpy.data.images.remove(image)
            layer = obj.data.uv_layers.get(UV_NAME)
            if layer is not None:
                obj.data.uv_layers.remove(layer)
        return {'FINISHED'}


def socket_is_lit(socket):
    """Whether a colour or strength input lets light through: linked, or not black / zero"""
    if socket is None:
        return False
    if socket.is_linked:
        return True
    value = socket.default_value
    if isinstance(value, (int, float)):
        return value > 0.0
    return any(v > 0.0 for v in value[:3])


def is_emissive(material):
    """A lamp: its Principled BSDF has emission, or an Emission shader is wired into it"""
    if material is None or material.node_tree is None:
        return False
    for node in material.node_tree.nodes:
        if node.type == 'BSDF_PRINCIPLED':
            # "Emission" until Blender 4.0, black by default; "Emission Color" since, white with
            # a strength of 0
            colour = node.inputs.get("Emission Color") or node.inputs.get("Emission")
            strength = node.inputs.get("Emission Strength")
            if socket_is_lit(colour) and (strength is None or socket_is_lit(strength)):
                return True
        elif node.type == 'EMISSION' and any(output.is_linked for output in node.outputs):
            if socket_is_lit(node.inputs.get("Color")) and socket_is_lit(node.inputs.get("Strength")):
                return True
    return False


def lit_faces(mesh):
    """The faces a lightmap is for: all but the lamps'"""
    lamps = {i for i, material in enumerate(mesh.materials) if is_emissive(material)}
    return {p.index for p in mesh.polygons if p.material_index not in lamps}


def lightmap_material(image):
    material = bpy.data.materials.new("Lightmap_" + image.name)
    if material.node_tree is None:
        material.use_nodes = True
    tree = material.node_tree
    shader = next((n for n in tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
    if shader is None:
        shader = tree.nodes.new('ShaderNodeBsdfPrincipled')
        output = next((n for n in tree.nodes if n.type == 'OUTPUT_MATERIAL'), None) or tree.nodes.new('ShaderNodeOutputMaterial')
        tree.links.new(shader.outputs['BSDF'], output.inputs['Surface'])
    texture = tree.nodes.new('ShaderNodeTexImage')
    texture.image = image
    tree.links.new(texture.outputs['Color'], shader.inputs['Base Color'])
    return material


def lightmap_copy(obj):
    """The object once more, drawn with its lightmap: one material, the lightmap UV map alone.
    None when it is all lamp"""
    lit = lit_faces(obj.data)
    if not lit:
        return None
    copy = obj.copy()
    copy.data = obj.data.copy()
    copy.name = COPY_PREFIX + obj.name
    mesh = copy.data
    if len(lit) < len(mesh.polygons):
        keep_faces(mesh, lit)
    while len(mesh.uv_layers) > 1:
        other = next(layer for layer in mesh.uv_layers if layer.name != UV_NAME)
        mesh.uv_layers.remove(other)
    # a mesh that was given glTF material variants is exported with the materials those name,
    # whatever its slots have
    for name in ("gltf2_variant_mesh_data", "gltf2_variant_default_materials"):
        variants = getattr(mesh, name, None)
        if variants is not None:
            variants.clear()
    material = lightmap_material(obj.ge_lightmap)
    mesh.materials.clear()
    mesh.materials.append(material)
    for collection in obj.users_collection:
        collection.objects.link(copy)
    return copy, mesh, material


class ExportLevel(bpy.types.Operator, ExportHelper):
    """Export the scene as .glb with its lightmaps, for `eurochef ge new-map`"""
    bl_idname = "ge.export_level"
    bl_label = "Export Level (.glb)"

    filename_ext = ".glb"
    filter_glob: StringProperty(default="*.glb", options={'HIDDEN'})

    use_selection: BoolProperty(name="Selected Objects Only", default=False)
    apply_modifiers: BoolProperty(name="Apply Modifiers", default=True)
    vertex_colours: BoolProperty(
        name="Vertex Colours", description="Export each mesh's active colour attribute, painted light", default=True
    )

    @classmethod
    def poll(cls, context):
        return context.mode == 'OBJECT'

    def execute(self, context):
        if self.use_selection:
            sources = [o for o in context.selected_objects if has_lightmap(o)]
        else:
            sources = [o for o in context.scene.objects if has_lightmap(o) and o.visible_get()]

        made = []
        try:
            for obj in sources:
                copy = lightmap_copy(obj)
                if copy is None:
                    continue
                made.append(copy)
                if self.use_selection:
                    made[-1][0].select_set(True)

            options = dict(
                filepath=self.filepath,
                export_format='GLB',
                use_selection=self.use_selection,
                export_apply=self.apply_modifiers,
            )
            try:
                if self.vertex_colours:
                    bpy.ops.export_scene.gltf(export_vertex_color='ACTIVE', **options)
                else:
                    bpy.ops.export_scene.gltf(**options)
            except TypeError:
                # an exporter that doesn't know that option yet
                bpy.ops.export_scene.gltf(**options)
        finally:
            for copy, mesh, material in made:
                bpy.data.objects.remove(copy)
                bpy.data.meshes.remove(mesh)
                bpy.data.materials.remove(material)

        self.report({'INFO'}, "{} with {} lightmap(s)".format(os.path.basename(self.filepath), len(made)))
        return {'FINISHED'}


class LightmapPanel(bpy.types.Panel):
    bl_idname = "VIEW3D_PT_ge_lightmaps"
    bl_label = "Lightmaps"
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = "GoldenEye"

    def draw(self, context):
        layout = self.layout
        settings = context.scene.ge_lightmaps

        column = layout.column(align=True)
        column.prop(settings, "size")
        column.prop(settings, "samples")
        column.prop(settings, "denoise")
        column.prop(settings, "margin")
        column.prop(settings, "exposure")
        column.prop(settings, "unwrap")
        column.prop(settings, "island_margin")
        column.prop(settings, "unwrap_again")
        column.prop(settings, "hide_sky")
        column.prop(settings, "split")
        row = column.row()
        row.enabled = settings.split
        row.prop(settings, "density")

        meshes = [o for o in context.selected_objects if o.type == 'MESH']
        with_lightmap = sum(1 for o in meshes if has_lightmap(o))
        layout.label(text="Selected: {} mesh(es), {} with a lightmap".format(len(meshes), with_lightmap))
        layout.operator(BakeLightmaps.bl_idname, icon='RENDER_STILL')
        layout.operator(RemoveLightmaps.bl_idname, icon='TRASH')

        obj = context.active_object
        if obj is not None and obj.type == 'MESH':
            layout.prop(obj, "ge_lightmap", text="Image")

        layout.separator()
        in_scene = sum(1 for o in context.scene.objects if has_lightmap(o))
        layout.label(text="Scene: {} object(s) with a lightmap".format(in_scene))
        layout.operator(ExportLevel.bl_idname, icon='EXPORT')


CLASSES = (Settings, BakeLightmaps, RemoveLightmaps, ExportLevel, LightmapPanel)


def register():
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.Scene.ge_lightmaps = PointerProperty(type=Settings)
    bpy.types.Object.ge_lightmap = PointerProperty(
        type=bpy.types.Image, name="Lightmap", description="This object's baked light, drawn over its textures in the game"
    )


def unregister():
    del bpy.types.Object.ge_lightmap
    del bpy.types.Scene.ge_lightmaps
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()
