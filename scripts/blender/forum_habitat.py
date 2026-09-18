"""Dense garden habitat and geological space-disc, authored in real metres.

Executed inside the landscape builder. Inspiration and scope are recorded in
docs/design/solar-forum/REALISM_REFERENCES.md; these are original scene meshes.
"""
strata=[mat('Basalt stratum '+str(i),c,rough=.9) for i,c in enumerate([
    (.42,.33,.22),(.30,.30,.32),(.20,.15,.11),(.55,.50,.42)])]
# Match the land's irregular outer silhouette, then taper to a deep mineral keel.
profile=[(174,-4),(172,-8),(167,-13),(158,-19),(144,-26),(122,-33),(88,-40),(42,-46),(0,-48)]
# Reuse the exact terrain boundary. Every adjacent stratum shares its full
# edge; independently perturbed edges previously left visible black cracks.
coast_boundary=[tuple(v.co) for v in bpy.data.objects['Sculpted coastal ground'].data.vertices[-160:]]
stratum_rings=[coast_boundary]
for ring_index,(radius,height) in enumerate(profile[1:],1):
    points=[]
    for i in range(160):
        angle=i*math.tau/160
        wave=1+.035*math.sin(angle*7)+.023*math.cos(angle*13)
        points.append((radius*wave*math.cos(angle),radius*wave*math.sin(angle),height+(math.sin(angle*17+ring_index)*.55 if radius else 0)))
    stratum_rings.append(points)
for layer,(upper,lower) in enumerate(zip(stratum_rings,stratum_rings[1:])):
    vv=upper+lower;n=len(upper)
    # Clockwise viewed from above: exterior normals point away from the disc.
    ff=[(i,i+n,(i+1)%n+n,(i+1)%n) for i in range(n)]
    me=bpy.data.meshes.new('Exposed geological strata');me.from_pydata(vv,[],ff);me.update()
    ob=bpy.data.objects.new('Exposed geological strata',me);scene.collection.objects.link(ob);finish(ob,ob.name,strata[layer%4])
# Visible engineering gives the floating land mass thickness and a legible scale.
for radius,z in [(164,-14),(139,-28),(82,-42)]:
    arc_band('Habitat structural compression ring',radius-1,radius+1,z,1.2,bronze,segments=192)
    arc_band('Recessed service light',radius+.92,radius+1.03,z+.08,.16,cyan,segments=192)
for i in range(24):
    a=i*math.tau/24
    for (ra,za),(rb,zb) in zip([(162,-15),(138,-29),(80,-43)],[(138,-29),(80,-43),(20,-48)]):
        branch((ra*math.cos(a),ra*math.sin(a),za),(rb*math.cos(a),rb*math.sin(a),zb),.62,bronze)
    box('Underside service cassette',(150*math.cos(a),150*math.sin(a),-22),(6,3,1.5),dark,a)
cyl('Geodisc mineral keel',(0,0,-46),22,5,dark,top=29,verts=96)
arc_band('Keel luminous rim',22,22.2,-47.1,.22,cyan,segments=128)

# The early landscape pass placed eight repeated stepped pyramids here.  Keep
# the legacy recipe in source history for reference, but disable its placement:
# forum_towers.py now owns the three varied urban rooms required by Job 12.
habitat_centers=[]
for index,(cx,cy) in enumerate(habitat_centers):
    label=['Archive gardens','Research studios','Civic residences','Learning ateliers'][index%4]
    # Podium covers the sculpted terrain locally; ground entrances stay at datum.
    detailed_box(label+' foundation',(cx,cy,-.48),(32,30,1),stone,bevel=.10)
    floors=3 if index<4 else 5
    for level in range(floors):
        z=level*3.6;w=29-level*3.5;d=25-level*2.4
        detailed_box(label+' terrace slab',(cx,cy,z+.16),(w+2,d+2,.32),pale,bevel=.06)
        # Genuine recessed facades: individual piers, glass panels and interiors.
        box(label+' rear wall',(cx,cy+d/2-1,z+1.8),(w,.35,3.3),stone)
        for side in [-1,1]:box(label+' side wall',(cx+side*(w/2-.3),cy,z+1.8),(.35,d-2,3.3),pale)
        for bay in range(int(w/3)):
            x=cx-w/2+1.5+bay*3;y=cy-d/2+1
            detailed_box('Terrace facade mullion',(x-1.4,y,z+1.8),(.10,.18,3.2),bronze,bevel=.015)
            if bay!=int(w/6):box('Recessed atelier glazing',(x,y+.06,z+1.8),(2.65,.035,2.85),glass)
            box('Facade sill',(x,y-.08,z+.43),(2.8,.35,.13),stone)
            for slat in range(4):box('Solar shading louvre',(x,y-.65,z+3.1-slat*.15),(2.8,.35,.055),timber)
        # A real furnished strip is visible through the glass and open doorway.
        for dx in [-w*.25,w*.25]:
            detailed_box('Atelier shared desk',(cx+dx,cy-2,z+1.1),(3,1.2,.15),timber)
            for off in [-1.1,1.1]:box('Desk trestle',(cx+dx+off,cy-2,z+.58),(.1,.8,1),bronze)
            for off in [-.8,.8]:
                box('Studio seat',(cx+dx+off,cy-3,z+.55),(.5,.5,.09),timber)
                box('Studio seat back',(cx+dx+off,cy-3.22,z+.88),(.5,.08,.6),timber)
            box('Archive cabinet',(cx+dx,cy+d/2-1.8,z+1.2),(2.8,.65,2),timber)
            for k in range(9):box('Archive document spine',(cx+dx-1.1+k*.25,cy+d/2-2.17,z+1.4),(.12,.12,.7+(k%3)*.07),clay if k%2 else pale)
        # Planters, balcony rails and exposed drainage give the roof edge depth.
        for side in [-1,1]:
            x=cx+side*(w/2+.2)
            detailed_box('Balcony planter trough',(x,cy,z+.63),(1.1,d-1,.7),stone,bevel=.045)
            for yy in range(-int(d/2)+2,int(d/2)-1,2):leaf_spray('Terrace herb garden',(x,cy+yy,z+1.03),.65,24)
            branch((x,cy+d/2,z+.4),(x,cy+d/2,z+3.7),.055,bronze)
        rail_segment('Garden balcony rail',(cx-w/2,cy-d/2-.65,z+.32),(cx+w/2,cy-d/2-.65,z+.32))
    roof=floors*3.6;rw=29-(floors-1)*3.5;rd=25-(floors-1)*2.4
    detailed_box('Roof garden slab',(cx,cy,roof),(rw+2,rd+2,.3),pale,bevel=.055)
    for dx in [-rw*.35,rw*.35]:
        for dy in [-rd*.3,rd*.3]:
            cyl('Roof orchard planter',(cx+dx,cy+dy,roof+.45),1,.8,clay,verts=20)
            branch((cx+dx,cy+dy,roof+.8),(cx+dx,cy+dy,roof+3),.09,wood)
            for h in [2,2.6,3.1]:leaf_spray('Roof orchard canopy',(cx+dx,cy+dy,roof+h),1.2,48)
    for dx in [-rw*.42,rw*.42]:cyl('Photovoltaic pergola column',(cx+dx,cy,roof+1.8),.1,3.6,bronze,verts=12)
    for k in range(7):
        box('Roof photovoltaic blade',(cx,cy-3+k,roof+3.6),(rw, .72,.12),solar)
    # Broad entrance stair, separate from all established agent patrol corridors.
    for step in range(24):
        zz=(step+1)*.15
        box('Garden external stair',(cx+16,cy-10+step*.3,zz/2),(2.2,.31,zz),pale)
    box('Garden stair upper landing',(cx+14,cy-2.8,3.45),(6,2.4,.3),pale)
    rail_segment('Garden stair rail',(cx+17,cy-10,0),(cx+17,cy-2.8,3.6))
