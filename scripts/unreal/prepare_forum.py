"""Copy installed Epic template dependencies locally; never commit Epic content."""
import os, shutil, json
from pathlib import Path
root=Path(__file__).resolve().parents[2]
engine=Path(os.environ.get('FORUM_ENGINE','/Users/Shared/Epic Games/UE_5.8'))
content=root/'unreal/SolarForum/Content'
for source,target in [
    (engine/'Templates/TP_FirstPersonBP/Content/FirstPerson',content/'FirstPerson'),
    (engine/'Templates/TemplateResources/High/Characters/Content',content/'Characters'),
    (engine/'Templates/TemplateResources/High/Input/Content',content/'Input'),
]:
    assert source.is_dir(), source
    shutil.copytree(source,target,dirs_exist_ok=True)
src=engine/'Templates/TP_FirstPersonBP/Config/DefaultInput.ini'
shutil.copy(src,root/'unreal/SolarForum/Config/DefaultInput.ini')

(root/'docs/design/solar-forum/unreal-import.json').write_text(json.dumps({'status':'pending'})+'\n')
