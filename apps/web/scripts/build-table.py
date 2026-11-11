"""Blender 5: reproducible furniture, card stock and photographic hero.
Run: blender --background --python scripts/build-table.py
"""
import bpy, math, sys
from pathlib import Path
from mathutils import Vector
ROOT=Path(__file__).resolve().parent.parent
OUT=ROOT/'static/art'; OUT.mkdir(exist_ok=True)
bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete(use_global=False)
def mat(name,color,rough=.5,metal=0):
 m=bpy.data.materials.new(name);m.diffuse_color=(*color,1);m.use_nodes=True
 p=m.node_tree.nodes.get('Principled BSDF');p.inputs['Base Color'].default_value=(*color,1);p.inputs['Roughness'].default_value=rough;p.inputs['Metallic'].default_value=metal
 return m
leather=mat('Forest leather',(.018,.046,.031),.7)
wood=mat('Warm walnut',(.085,.033,.014),.35)
brass=mat('Aged brass',(.38,.23,.08),.3,.7)
stock=mat('Ivory linen stock',(.91,.89,.80),.66)
felt=mat('Bottle green baize',(.018,.075,.046),.94)
def texture(name):
 m=mat(name,(1,1,1),.65);ns=m.node_tree.nodes; tex=ns.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(OUT/'cards'/f'{name}.png'));m.node_tree.links.new(tex.outputs['Color'],ns.get('Principled BSDF').inputs['Base Color']);return m
def cylinder(name,r,depth,z,material,verts=128):
 bpy.ops.mesh.primitive_cylinder_add(vertices=verts,radius=r,depth=depth,location=(0,0,z));o=bpy.context.object;o.name=name;o.data.materials.append(material)
 bevel=o.modifiers.new('Soft edge','BEVEL');bevel.width=.004;bevel.segments=3
 bpy.context.view_layer.objects.active=o;bpy.ops.object.modifier_apply(modifier=bevel.name)
 for p in o.data.polygons:p.use_smooth=True
 return o
def torus(name,r,t,z,material):
 bpy.ops.mesh.primitive_torus_add(major_segments=128,minor_segments=12,major_radius=r,minor_radius=t,location=(0,0,z));o=bpy.context.object;o.name=name;o.data.materials.append(material)
 for p in o.data.polygons:p.use_smooth=True
 return o
base=cylinder('Walnut base',.472,.039,-.030,wood)
# Sculpted cushion profile avoids the floating rubber-hoop silhouette.
profile=[(.438,-.025),(.440,-.003),(.446,.006),(.458,.008),(.471,.003),(.480,-.009),(.476,-.031),(.453,-.036)]
verts=[(r*math.cos(i*math.tau/128),r*math.sin(i*math.tau/128),z) for i in range(128) for r,z in profile]
faces=[(i*8+j,((i+1)%128)*8+j,((i+1)%128)*8+(j+1)%8,i*8+(j+1)%8) for i in range(128) for j in range(8)]
mesh=bpy.data.meshes.new('Cushion');mesh.from_pydata(verts,[],faces);mesh.update()
rail=bpy.data.objects.new('Padded leather rail',mesh);bpy.context.collection.objects.link(rail);rail.data.materials.append(leather)
for poly in mesh.polygons:poly.use_smooth=True
bpy.ops.object.select_all(action='DESELECT');rail.select_set(True);bpy.context.view_layer.objects.active=rail
bpy.ops.object.mode_set(mode='EDIT');bpy.ops.mesh.select_all(action='SELECT');bpy.ops.uv.smart_project(island_margin=.025);bpy.ops.object.mode_set(mode='OBJECT')
trim=torus('Brass piping',.483,.0015,-.022,brass)

def bake_surface(obj, material, kind):
 ns=material.node_tree.nodes; links=material.node_tree.links;p=ns.get('Principled BSDF')
 coord=ns.new('ShaderNodeTexCoord');noise=ns.new('ShaderNodeTexNoise');noise.inputs['Scale'].default_value=210 if kind=='leather' else 7
 noise.inputs['Detail'].default_value=3
 mapping=ns.new('ShaderNodeVectorMath');mapping.operation='MULTIPLY';mapping.inputs[1].default_value=(1,1,1) if kind=='leather' else (3,3,90)
 links.new(coord.outputs['Generated'],mapping.inputs[0]);links.new(mapping.outputs[0],noise.inputs['Vector'])
 ramp=ns.new('ShaderNodeValToRGB')
 colors=((.009,.021,.014,1),(.042,.071,.052,1)) if kind=='leather' else ((.045,.017,.007,1),(.19,.073,.025,1))
 for e,c in zip(ramp.color_ramp.elements,colors):e.color=c
 links.new(noise.outputs['Fac'],ramp.inputs[0]);links.new(ramp.outputs[0],p.inputs['Base Color'])
 bump=ns.new('ShaderNodeBump');bump.inputs['Strength'].default_value=.22;bump.inputs['Distance'].default_value=.0006
 links.new(noise.outputs['Fac'],bump.inputs['Height']);links.new(bump.outputs['Normal'],p.inputs['Normal'])
 bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
 scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8
 imgs=[]
 for mode in ['DIFFUSE','NORMAL']:
  im=bpy.data.images.new(kind+mode,512,512);im.colorspace_settings.name='sRGB' if mode=='DIFFUSE' else 'Non-Color'
  target=ns.new('ShaderNodeTexImage');target.image=im;ns.active=target
  bpy.ops.object.bake(type=mode,pass_filter={'COLOR'},margin=8)
  imgs.append(target)
 links.new(imgs[0].outputs['Color'],p.inputs['Base Color'])
 nm=ns.new('ShaderNodeNormalMap');links.new(imgs[1].outputs['Color'],nm.inputs['Color']);links.new(nm.outputs['Normal'],p.inputs['Normal'])
 for node in imgs:node.image.pack()
bake_surface(base,wood,'wood')
bake_surface(rail,leather,'leather')

# Export static furniture independently; felt uses the runtime's shared textile texture.
bpy.ops.object.select_all(action='DESELECT')
for o in [base,rail,trim]:o.select_set(True)
bpy.ops.export_scene.gltf(filepath=str(OUT/'table.glb'),export_format='GLB',use_selection=True)
cylinder('Baize',.449,.008,-.006,felt)
# Rounded rectangular cardstock, actual thickness, shared low-poly silhouette.
def card(name,x,y,z,angle,face):
 w=.0635;h=.0889;r=.003
 outline=[]
 for cx,cy,a in [(w/2-r,h/2-r,0),(-w/2+r,h/2-r,90),(-w/2+r,-h/2+r,180),(w/2-r,-h/2+r,270)]:
  for j in range(7):
   t=math.radians(a+j*15);outline.append((cx+r*math.cos(t),cy+r*math.sin(t)))
 n=len(outline);v=[(a,b,c) for c in [-.00032,.00032] for a,b in outline]
 faces=[tuple(range(n-1,-1,-1)),tuple(range(n,2*n))]+[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
 mesh=bpy.data.meshes.new(name);mesh.from_pydata(v,[],faces);mesh.update()
 o=bpy.data.objects.new(name,mesh);bpy.context.collection.objects.link(o);o.data.materials.append(stock);o.data.materials.append(texture(face));o.data.polygons[1].material_index=1
 uv=mesh.uv_layers.new()
 for p in mesh.polygons:
  for li in p.loop_indices:
   co=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=(co.x/w+.5,co.y/h+.5)
 o.location=(x,y,z);o.rotation_euler.z=angle
 return o
# Export a unit-height stock mesh in XY after glTF's axis conversion.
prototype=card('CardStock',0,0,2,0,'back')
prototype.data.materials.clear();prototype.data.materials.append(stock)
for p in prototype.data.polygons:p.material_index=0
prototype.location=(0,0,0);prototype.rotation_euler.x=math.pi/2;prototype.scale=(1/.0889,1/.0889,1/.0889)
bpy.ops.object.select_all(action='DESELECT');prototype.select_set(True);bpy.context.view_layer.objects.active=prototype
bpy.ops.object.transform_apply(location=True,rotation=True,scale=True)
bpy.ops.export_scene.gltf(filepath=str(OUT/'card-stock.glb'),export_format='GLB',use_selection=True)
bpy.data.objects.remove(prototype,do_unlink=True)
# Shared ivory dealer token, modeled and lettered in Blender.
puck=cylinder('DealerToken',.014,.003,.0015,stock,48)
bpy.ops.object.text_add(location=(0,0,.0031));label=bpy.context.object;label.data.body='D';label.data.align_x='CENTER';label.data.align_y='CENTER';label.data.size=.014;label.data.extrude=.00003;label.data.materials.append(leather)
bpy.ops.object.convert(target='MESH')
bpy.ops.object.select_all(action='DESELECT');puck.select_set(True);label.select_set(True)
bpy.ops.export_scene.gltf(filepath=str(OUT/'dealer.glb'),export_format='GLB',use_selection=True)
bpy.data.objects.remove(puck,do_unlink=True);bpy.data.objects.remove(label,do_unlink=True)
if '--runtime-only' in sys.argv:
 bpy.ops.wm.save_as_mainfile(filepath=str(ROOT/'assets/game-table.blend'))
 print('RUNTIME_MODELS_COMPLETE');sys.exit(0)
# Hero: tightly composed overhead fan with a real stacked deck.
for i in range(10):card('Deck leaf',-.073,.027,.002+i*.00075,math.radians(-13),'back')
for i,face in enumerate(['AS','KS','QH','JD','JH']):
 a=math.radians((i-2)*-13)
 card(face,(i-2)*.027,-.025+abs(i-2)*.008,.015+i*.002,a,face)
# Dealer puck gives scale and a familiar physical accent.
puck=cylinder('Dealer ivory',.013,.004,.003,stock,48);puck.location.x=.093;puck.location.y=.066
bpy.ops.object.text_add(location=(.093,.066,.0052));label=bpy.context.object;label.data.body='D';label.data.align_x='CENTER';label.data.align_y='CENTER';label.data.size=.014;label.data.extrude=.00005;label.data.materials.append(leather)
# Save editable full source before rendering.
scene=bpy.context.scene
scene.render.engine='CYCLES';scene.cycles.samples=48;scene.cycles.use_denoising=True
scene.world.color=(.08,.08,.08)
def area(name,pos,power,size,color):
 bpy.ops.object.light_add(type='AREA',location=pos);o=bpy.context.object;o.name=name;o.data.energy=power;o.data.shape='DISK';o.data.size=size;o.data.color=color;o.rotation_euler=(Vector((0,0,0))-o.location).to_track_quat('-Z','Y').to_euler()
area('Large softbox',(-.18,-.12,.45),2.5,.3,(1,.92,.8));area('Cool fill',(.2,.2,.3),1,.25,(.79,.89,1))
bpy.ops.object.camera_add(location=(.03,-.24,.48));cam=bpy.context.object;cam.rotation_euler=(Vector((0,.005,.015))-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.type='ORTHO';cam.data.ortho_scale=.285;scene.camera=cam
scene.render.resolution_x=1200;scene.render.resolution_y=1000;scene.render.resolution_percentage=100
scene.view_settings.view_transform='AgX'
# The hero shows the card still life on felt. No runtime lights or GPU animation required.
scene.render.image_settings.file_format='WEBP';scene.render.image_settings.color_mode='RGB';scene.render.image_settings.quality=90
scene.render.filepath=str(OUT/'euchre-hero.webp')
bpy.ops.wm.save_as_mainfile(filepath=str(ROOT/'assets/euchre-table.blend'))
bpy.ops.render.render(write_still=True)
print('BLENDER_ASSETS_COMPLETE')
