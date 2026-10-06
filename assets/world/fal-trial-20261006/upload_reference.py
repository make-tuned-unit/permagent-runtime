"""PUT a local reference using a private receipt from FAL prepare_upload.
No key required; the receipt is deleted after a successful verified-HTTPS PUT.
"""
import ipaddress,json,pathlib,subprocess,sys,urllib.parse,urllib.request
receipt_path=pathlib.Path(sys.argv[1]);source=pathlib.Path(sys.argv[2]);receipt=json.loads(receipt_path.read_text())
url=receipt['upload_url'];parsed=urllib.parse.urlsplit(url)
if parsed.scheme!='https' or not parsed.hostname or receipt['method']!='PUT':raise ValueError('Expected HTTPS PUT receipt')
request=urllib.request.Request('https://1.1.1.1/dns-query?'+urllib.parse.urlencode({'name':parsed.hostname,'type':'A'}),headers={'accept':'application/dns-json'})
with urllib.request.urlopen(request,timeout=15) as response:dns=json.load(response)
address=str(ipaddress.IPv4Address(next(a['data'] for a in dns['Answer'] if a['type']==1)))
headers=sum((['--header',key+': '+value] for key,value in receipt['headers'].items()),[])
result=subprocess.run(['/usr/bin/curl','--fail','--silent','--show-error','--proto','=https','--connect-timeout','15','--max-time','90','--resolve',parsed.hostname+':443:'+address,*headers,'--upload-file',str(source),'--config','-','--write-out','%{http_code}'],input='url = '+json.dumps(url)+'\n',text=True,capture_output=True)
status=result.stdout[-3:]
if result.returncode or not status.startswith('2'):raise RuntimeError('Upload failed; curl exit '+str(result.returncode)+'; HTTP '+status)
receipt_path.unlink();print('Reference uploaded successfully; HTTP '+status)
