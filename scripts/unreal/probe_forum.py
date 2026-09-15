import unreal, json
from pathlib import Path
out=Path(__file__).resolve().parents[2]/'docs/design/solar-forum/unreal-probe.json'
names=['AssetImportTask','FbxImportUI','EditorAssetLibrary','EditorLevelLibrary','StaticMeshActor','PlayerStart','DefaultPawn','GameModeBase','NavigationSystemV1','NavMeshBoundsVolume','EditorActorSubsystem']
out.write_text(json.dumps({'engine':unreal.SystemLibrary.get_engine_version(),'api':{n:hasattr(unreal,n) for n in names}},indent=2))
unreal.log('SOLAR_FORUM_PROBE_OK')
