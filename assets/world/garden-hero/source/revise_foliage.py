"""Bounded botanical mesh authoring. Requires the approved RGBA atlas path/layout."""
import bpy,math,random,os,json,shutil,hashlib,struct
from pathlib import Path
from mathutils import Vector,Quaternion
BASE=Path(__file__).resolve().parent.parent
OUT=BASE/'exports'/'foliage-v3b';OUT.mkdir(exist_ok=True)
atlas=Path(os.environ['WORLD_LEAF_ATLAS']);cols=int(os.environ.get('WORLD_ATLAS_COLS','2'));rows=int(os.environ.get('WORLD_ATLAS_ROWS','2'))
assert atlas.exists();random.seed(61006)
if shutil.disk_usage(BASE).free<3*1024**3:raise SystemExit('Disk floor')
bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
kitbackup=BASE/'source'/'hero-kit-v2.glb'
if not kitbackup.exists():shutil.copyfile(BASE/'exports'/'hero-kit.glb',kitbackup)
image=bpy.data.images.load(str(atlas));image.pack()
assert image.channels==4,'Requires true RGBA atlas'
assert image.pixels[3]<.01,'Atlas corner must be transparent'
print('ATLAS_RGBA_CONFIRMED',image.size[:],image.channels,flush=True)
leafmat=bpy.data.materials.new('Leaf • approved RGBA atlas');leafmat.use_nodes=True;leafmat.use_backface_culling=False
p=leafmat.node_tree.nodes.get('Principled BSDF');p.inputs['Roughness'].default_value=.74;p.inputs['Specular IOR Level'].default_value=.25
t=leafmat.node_tree.nodes.new('ShaderNodeTexImage');t.image=image;leafmat.node_tree.links.new(t.outputs['Color'],p.inputs['Base Color']);leafmat.node_tree.links.new(t.outputs['Alpha'],p.inputs['Alpha'])
bark=bpy.data.materials.new('Bark • cinnamon fine branching');bark.diffuse_color=(.16,.084,.035,1);bark.use_nodes=True;p=bark.node_tree.nodes.get('Principled BSDF');p.inputs['Base Color'].default_value=bark.diffuse_color;p.inputs['Roughness'].default_value=.91
stats={};roots={}
def make_mesh(name,v,f,uvs,material,root):
 me=bpy.data.meshes.new(name);me.from_pydata(v,[],f);me.materials.append(material)
 for face in me.polygons:face.use_smooth=True
 if uvs:
  layer=me.uv_layers.new(name='AtlasUV')
  for poly in me.polygons:
   for li in poly.loop_indices:layer.data[li].uv=uvs[me.loops[li].vertex_index]
 o=bpy.data.objects.new(name,me);bpy.context.collection.objects.link(o);o.parent=root;return o
class Plant:
 def __init__(self,name):
  self.v=[];self.f=[];self.lv=[];self.lf=[];self.uv=[];self.leaves=0;self.branches=0
  self.root=bpy.data.objects.new(name,None);bpy.context.collection.objects.link(self.root);roots[name]=self.root
 def branch(self,points,r0,r1,sides=7,radii=None):
  points=[Vector(p) for p in points];start=len(self.v)
  for i,p in enumerate(points):
   tangent=points[min(i+1,len(points)-1)]-points[max(0,i-1)];rot=tangent.to_track_quat('Z','Y');radius=radii[i] if radii else r0+(r1-r0)*i/(len(points)-1)
   for k in range(sides):self.v.append(tuple(p+rot@Vector((radius*math.cos(k*math.tau/sides),radius*math.sin(k*math.tau/sides),0))))
  for i in range(len(points)-1):
   for k in range(sides):a=start+i*sides+k;b=start+i*sides+(k+1)%sides;self.f.append((a,b,b+sides,a+sides))
  self.f.extend([tuple(start+i for i in reversed(range(sides))),tuple(start+(len(points)-1)*sides+i for i in range(sides))]);self.branches+=1
 def curved_branch(self,a,b,r0,r1,bend=.12):
  a,b=Vector(a),Vector(b);pts=[a.lerp(b,i/4)+Vector((0,0,math.sin(i*math.pi/4)*bend)) for i in range(5)];self.branch(pts,r0,r1);return pts
 def leaf(self,p,d,length,cell):
  # Three curled/twisted profiles, expanded into one batched mesh per asset.
  profile=random.randrange(3);rot=Vector(d).to_track_quat('X','Z')@Quaternion(Vector((1,0,0)),random.uniform(-1.25,1.25));p=Vector(p);start=len(self.lv);cellx=cell%cols;celly=cell//cols
  curl=[.15,.26,-.18][profile];twist=random.uniform(-.20,.20);width=random.uniform(.43,.65)
  for i in range(5):
   u=i/4
   for j in range(3):
    v=j/2;z=math.sin(u*math.pi)*curl+abs(v-.5)*.18+twist*(v-.5)*u
    self.lv.append(tuple(p+rot@Vector((u*length,(v-.5)*length*width,z*length))))
    # UV pads cells slightly; imagery controls final leaf outline through alpha MASK.
    self.uv.append(((cellx+.025+v*.95)/cols,(celly+.025+u*.95)/rows))
  for i in range(4):
   for k in range(2):a=start+i*3+k;self.lf.append((a,a+1,a+4,a+3))
  self.leaves+=1
 def twig(self,a,d,length,leaf_length,count=8):
  a=Vector(a);d=Vector(d).normalized();b=a+d*length;pts=self.curved_branch(a,b,.009,.002,.045)
  for i in range(count):
   f=(i+.5)/count;q=a.lerp(b,f)+Vector((0,0,math.sin(f*math.pi)*.045));phi=i*2.399
   direction=(d*.4+Vector((math.cos(phi),math.sin(phi),random.uniform(-.55,.65)))).normalized()
   # Exclude bottom-left young tile, whose opaque yellow fringe parent rejected.
   self.leaf(q,direction,leaf_length*random.uniform(.72,1.28),random.choice([1,2,3]))
 def finish(self):
  make_mesh(self.root.name+' • hierarchical woody structure',self.v,self.f,None,bark,self.root);make_mesh(self.root.name+' • curled atlas leaves',self.lv,self.lf,self.uv,leafmat,self.root)
  triangles=sum(len(f)-2 for f in self.f)+sum(len(f)-2 for f in self.lf)
  print('PLANT_BUDGET',self.root.name,triangles,'leaves',self.leaves,flush=True)
  stats[self.root.name]={'triangles_authored':triangles,'materials':2,'branches':self.branches,'placed_leaves':self.leaves,'leaf_prototype_profiles':3,'leaf_prototype_vertices':45,'leaf_prototype_triangles':48,'actual_gpu_instances':0,'storage':'expanded geometry batched into two mesh nodes; no instance-count claim','leaf_mesh_vertices':len(self.lv)}
  lowwood=[Vector(v) for v in self.v if v[2]<=2]
  stats[self.root.name]['blender_solid_bounds_below_2m']=[[min(v[k] for v in lowwood) for k in range(3)],[max(v[k] for v in lowwood) for k in range(3)]]
  stats[self.root.name]['max_radial_wood_extent_below_2m']=max(math.hypot(v.x,v.y) for v in lowwood)
  if self.root.name=='canopy':assert triangles<=35000
tree=Plant('canopy')
trunk=[(.065*math.sin(i*.11),.035*math.sin(i*.14),i*.20) for i in range(19)]
# Broad root collar narrows monotonically into the main stem; gentle continuous bend.
trunk_radii=[.27,.22,.185,.17,.159,.148,.138,.128,.118,.109,.099,.090,.081,.072,.063,.054,.045,.036,.026]
tree.branch(trunk,.27,.026,10,trunk_radii)
for k in range(7):
 phi=k*math.tau/7;tree.branch([(math.cos(phi)*.46,math.sin(phi)*.46,.008),(math.cos(phi)*.31,math.sin(phi)*.31,.085),(math.cos(phi)*.15,math.sin(phi)*.15,.17),(0,0,.25)],.012,.12,7)
for k in range(12):
 phi=k*2.399;z=1.6+k*.12;radius=random.uniform(.95,1.4);a=Vector((.025,0,z));b=Vector((math.cos(phi)*radius,math.sin(phi)*radius,z+random.uniform(.65,.95)))
 tree.curved_branch(a,b,.045,.013,.12)
 for j in range(4):
  f=.40+j*.18;q=a.lerp(b,f)+Vector((0,0,math.sin(f*math.pi)*.12));ang=phi+random.uniform(-1,1);end=q+Vector((math.cos(ang)*random.uniform(.35,.60),math.sin(ang)*random.uniform(.35,.60),random.uniform(.18,.45)))
  tree.curved_branch(q,end,.016,.006,.07)
  for h in range(3):
   frac=.40+h*.28;anchor=q.lerp(end,frac);direction=(math.cos(ang+h*1.6),math.sin(ang+h*1.6),random.uniform(.0,.65));tree.twig(anchor,direction,random.uniform(.38,.60),random.uniform(.27,.35))
tree.finish()
stats['canopy']['lowest_primary_branch_centerline_m']=1.6;stats['canopy']['lowest_primary_branch_radius_m']=.045;stats['canopy']['lowest_primary_branch_underside_m']=1.555;stats['canopy']['trunk_collar_base_radius_m']=.27;stats['canopy']['root_flare_tip_radius_m']=.46
shrub=Plant('shrub')
for k in range(12):
 phi=k*2.399;a=Vector((math.cos(phi)*.11,math.sin(phi)*.11,.025));b=Vector((math.cos(phi)*random.uniform(.32,.56),math.sin(phi)*random.uniform(.32,.56),random.uniform(.55,.90)));shrub.curved_branch(a,b,.016,.007,.05)
 for j in range(3):
  q=a.lerp(b,.40+j*.22);ang=phi+j*1.3;end=q+Vector((math.cos(ang)*.22,math.sin(ang)*.22,.16));shrub.curved_branch(q,end,.008,.003,.035)
  for h in range(2):shrub.twig(q.lerp(end,.60+h*.35),(math.cos(ang+h*1.7),math.sin(ang+h*1.7),.25),random.uniform(.20,.31),random.uniform(.15,.21),8)
shrub.finish()
for name,root in roots.items():
 backup=BASE/'source'/f'{name}-v1.glb'
 if not backup.exists():shutil.copyfile(BASE/'exports'/f'{name}.glb',backup)
 bpy.ops.object.select_all(action='DESELECT');root.select_set(True)
 for o in root.children:o.select_set(True)
 bpy.context.view_layer.objects.active=root;bpy.ops.export_scene.gltf(filepath=str(OUT/f'{name}.glb'),export_format='GLB',use_selection=True,export_apply=True)
 # Explicit MASK prevents alpha sorting and tests the exact runtime-facing GLB.
 path=OUT/f'{name}.glb';data=path.read_bytes();size=struct.unpack_from('<I',data,12)[0];g=json.loads(data[20:20+size]);tail=data[20+size:]
 for m in g['materials']:
  if m['name'].startswith('Leaf'):m['alphaMode']='MASK';m['alphaCutoff']=.55;m['doubleSided']=True
 js=json.dumps(g,separators=(',',':')).encode();js+=b' '*((-len(js))%4);path.write_bytes(struct.pack('<4sII',b'glTF',2,20+len(js)+len(tail))+struct.pack('<II',len(js),0x4e4f534a)+js+tail)
bpy.ops.wm.save_as_mainfile(filepath=str(BASE/'source'/'foliage-v3b.blend'))
stats['atlas']={'path':str(atlas),'sha256':hashlib.sha256(atlas.read_bytes()).hexdigest(),'bytes':atlas.stat().st_size,'width':image.size[0],'height':image.size[1],'rgba_decoded_bytes':image.size[0]*image.size[1]*4,'grid':[cols,rows],'used_tiles_bottom_origin':[1,2,3],'alpha_mode':'MASK','alpha_cutoff':.55,'provenance':'parent supplied AI RGBA general broadleaf atlas, not species/scanned/full PBR; original PROVENANCE.md accompanies source; young tile omitted for visible fringe'}
(BASE/'foliage-v3b-stats.json').write_text(json.dumps(stats,indent=2))
# One scene diagnostic, two assets only. Export files are reimported to verify transforms/materials.
bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
for name,pos in [('canopy',(0,.7,0)),('shrub',(1.7,-.9,0))]:
 bpy.ops.import_scene.gltf(filepath=str(OUT/f'{name}.glb'));bpy.data.objects[name].location=pos
bpy.context.view_layer.update();coords=[o.matrix_world@Vector(c) for o in bpy.context.scene.objects if o.type=='MESH' for c in o.bound_box]
low=Vector([min(c[k] for c in coords) for k in range(3)]);high=Vector([max(c[k] for c in coords) for k in range(3)]);center=(low+high)/2
bpy.ops.object.camera_add(location=center+Vector((5,-8,3)));cam=bpy.context.object;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.type='ORTHO'
bpy.context.view_layer.update();inv=cam.matrix_world.inverted();projected=[inv@c for c in coords];xs=[c.x for c in projected];ys=[c.y for c in projected]
cam.data.ortho_scale=max(max(xs)-min(xs),max(ys)-min(ys))*1.10
# Shift optical centre to projected bound centre, covering canopy + shrub + root flare.
offset=cam.rotation_euler.to_quaternion()@Vector(((max(xs)+min(xs))/2,(max(ys)+min(ys))/2,0));cam.location+=offset
sc=bpy.context.scene;sc.camera=cam;sc.render.engine='CYCLES';sc.cycles.device='CPU';sc.cycles.samples=4;sc.cycles.use_denoising=False;sc.render.threads_mode='FIXED';sc.render.threads=1;sc.render.resolution_x=512;sc.render.resolution_y=512;sc.render.resolution_percentage=100
sc.world.use_nodes=True;sc.world.node_tree.nodes['Background'].inputs[0].default_value=(.34,.39,.44,1);sc.world.node_tree.nodes['Background'].inputs[1].default_value=.55
# One diagnostic with split light/dark backboards for cutout edge inspection.
for x,color in [(-2.8,(.035,.045,.055,1)),(2.8,(.68,.70,.67,1))]:
 mat=bpy.data.materials.new('Diagnostic backboard');mat.use_nodes=True;shader=mat.node_tree.nodes.get('Principled BSDF');shader.inputs['Base Color'].default_value=color;shader.inputs['Emission Color'].default_value=color;shader.inputs['Emission Strength'].default_value=.4
 bpy.ops.mesh.primitive_plane_add(size=1,location=(x,3.2,2.6),rotation=(math.pi/2,0,0));o=bpy.context.object;o.scale=(5.6,7,1);o.data.materials.append(mat)
for loc,power,color in [((-4,-4,7),1100,(1,.86,.72)),((3,2,6),900,(.72,.83,1))]:
 bpy.ops.object.light_add(type='AREA',location=loc);o=bpy.context.object;o.data.energy=power;o.data.size=4;o.data.color=color;o.rotation_euler=(Vector((0,0,2))-o.location).to_track_quat('-Z','Y').to_euler()
sc.render.filepath=str(BASE/'previews'/'foliage-v3b-export-diagnostic-512.png');sc.render.image_settings.file_format='PNG';bpy.ops.render.render(write_still=True);print('FOLIAGE_V3B_READY '+sc.render.filepath)
