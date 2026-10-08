from . import ge_lightmaps

bl_info = {
    "name": "Eurochef Utility",
    "author": "cohaereo",
    "description": "Lightmaps for GoldenEye 007 (Wii) levels: bake them and export the level",
    "blender": (4, 2, 0),
    "version": (0, 1, 0),
    "location": "3D View > Sidebar > GoldenEye",
    "category": "Import-Export"
}


def register():
    ge_lightmaps.register()


def unregister():
    ge_lightmaps.unregister()
