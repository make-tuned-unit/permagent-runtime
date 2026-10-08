#!/usr/bin/python3
"""Compile/sign the dedicated local test host. Never launches it."""
from pathlib import Path
import hashlib,json,plistlib,subprocess
ROOT=Path(__file__).resolve().parent
APP=ROOT/'Permagent Record Practice.app'
MACOS=APP/'Contents/MacOS'
MACOS.mkdir(parents=True,exist_ok=True)
source=ROOT/'main.swift'
binary=MACOS/'PermagentRecordPractice'
info={'CFBundleName':'Permagent Record Practice','CFBundleDisplayName':'Permagent Record Practice',
      'CFBundleIdentifier':'ai.permagent.record-practice-20261007','CFBundleExecutable':binary.name,
      'CFBundlePackageType':'APPL','CFBundleShortVersionString':'1.0','CFBundleVersion':'1',
      'LSMinimumSystemVersion':'13.0','NSHighResolutionCapable':True,
      'NSAppTransportSecurity':{'NSAllowsLocalNetworking':True}}
# Public narrowly scoped ATS allowance for local-network fixture HTTP only;
# no NSAllowsArbitraryLoads, no microphone/camera usage descriptions.
(APP/'Contents/Info.plist').write_bytes(plistlib.dumps(info))
command=['xcrun','swiftc','-parse-as-library','-swift-version','5','-target','arm64-apple-macosx13.0',str(source),'-o',str(binary),'-framework','AppKit','-framework','WebKit']
subprocess.run(command,check=True)
subprocess.run(['codesign','--force','--sign','-',str(APP)],check=True)
subprocess.run(['codesign','--verify','--strict',str(APP)],check=True)
report={'app':str(APP),'identifier':info['CFBundleIdentifier'],'command':command,
 'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
 'architecture':subprocess.check_output(['lipo','-archs',str(binary)],text=True).strip(),
 'url':'http://127.0.0.1:5489/','launched':False,'device_capture':'explicitly denied by public WKUIDelegate','playback':'user gesture required'}
(ROOT/'build-verification.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
