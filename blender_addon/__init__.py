from . import ge_lightmaps, ge_rooms, ge_transparent

bl_info = {
    "name": "Eurochef Utility",
    "author": "cohaereo",
    "description": "GoldenEye 007 (Wii) levels: lightmaps, rooms and portals, transparent geometry, export",
    "blender": (4, 2, 0),
    "version": (0, 1, 0),
    "location": "3D View > Sidebar > GoldenEye",
    "category": "Import-Export"
}

MODULES = (ge_lightmaps, ge_rooms, ge_transparent)


def register():
    for module in MODULES:
        module.register()


def unregister():
    for module in reversed(MODULES):
        module.unregister()
