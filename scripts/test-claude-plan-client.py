from pathlib import Path
import subprocess,threading,json,time,re,os,shlex
from http.server import ThreadingHTTPServer,BaseHTTPRequestHandler
ROOT=Path(__file__).resolve().parent.parent/'.scratch/claude-real-client'
ROOT.mkdir(parents=True,exist_ok=True)
old=ROOT/'hook-input.json'
if old.exists():old.unlink()
WORK=ROOT/'work';CONFIG=ROOT/'session-config';WORK.mkdir(exist_ok=True);CONFIG.mkdir(exist_ok=True)
HOOK=ROOT/'observe_hook.py'
HOOK.write_text('import sys,json\nfrom pathlib import Path\np=json.load(sys.stdin)\nPath(__file__).with_name("hook-input.json").write_text(json.dumps(p))\n')
settings=ROOT/'fixture-settings.json';settings.write_text(json.dumps({'hooks':{'PreToolUse':[{'matcher':'ExitPlanMode','hooks':[{'type':'command','command':'/usr/bin/python3 '+shlex.quote(str(HOOK)),'timeout':5}]}]}}))
requests=[]
class API(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_POST(self):
  data=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))));requests.append(data)
  if self.path.endswith('count_tokens'):
   b=json.dumps({'input_tokens':100}).encode();self.send_response(200);self.end_headers();self.wfile.write(b);return
  tools=[t.get('name') for t in data.get('tools',[])];(ROOT/'offered-tools.json').write_text(json.dumps(tools))
  raw=json.dumps(data,ensure_ascii=False)
  # Only seed a plan explicitly named by this isolated client's own system prompt.
  paths=re.findall(re.escape(str(CONFIG)) + r'/plans/[^"<>\\]+?\.md',raw)
  for path in paths:
   p=Path(path.rstrip('.,;)'))
   if p.is_absolute() and p.resolve().is_relative_to((CONFIG/'plans').resolve()):
    p.parent.mkdir(parents=True,exist_ok=True);p.write_text('# Controlled client plan\nReview only; no implementation.\n')
  block={'type':'tool_use','id':'toolu_fixture_plan','name':'ExitPlanMode','input':{}} if len(requests)==1 else {'type':'text','text':'Fixture finished after permission handling.'}
  stop='tool_use' if block['type']=='tool_use' else 'end_turn'
  msg={'id':'msg_fixture','type':'message','role':'assistant','model':data.get('model','fixture'),'content':[block],'stop_reason':stop,'stop_sequence':None,'usage':{'input_tokens':100,'output_tokens':10}}
  self.send_response(200);self.send_header('Content-Type','text/event-stream' if data.get('stream') else 'application/json');self.end_headers()
  if not data.get('stream'):self.wfile.write(json.dumps(msg).encode());return
  start={**msg,'content':[],'stop_reason':None,'usage':{'input_tokens':100,'output_tokens':0}}
  events=[('message_start',{'type':'message_start','message':start}),('content_block_start',{'type':'content_block_start','index':0,'content_block':block}),('content_block_stop',{'type':'content_block_stop','index':0}),('message_delta',{'type':'message_delta','delta':{'stop_reason':stop,'stop_sequence':None},'usage':{'output_tokens':10}}),('message_stop',{'type':'message_stop'})]
  for event,payload in events:self.wfile.write(('event: '+event+'\ndata: '+json.dumps(payload)+'\n\n').encode())
server=ThreadingHTTPServer(('127.0.0.1',0),API);threading.Thread(target=server.serve_forever,daemon=True).start()
env={'PATH':'/usr/bin:/bin','CLAUDE_CONFIG_DIR':str(CONFIG),'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1','DISABLE_TELEMETRY':'1','DISABLE_ERROR_REPORTING':'1','DISABLE_UPDATES':'1','ANTHROPIC_API_KEY':'fixture-only','ANTHROPIC_BASE_URL':'http://127.0.0.1:'+str(server.server_port)} # nosec: loopback fixture-only key, never a real credential
binary=ROOT/'runtime/node_modules/@anthropic-ai/claude-code-darwin-arm64/claude'
if not binary.is_file(): raise SystemExit('Install pinned official CLI in .scratch/claude-real-client/runtime first; see test documentation.')
subprocess.run(['/usr/bin/codesign','--verify','--strict',str(binary)],check=True,capture_output=True)
version=subprocess.run([str(binary),'--version'],cwd=WORK,env=env,check=True,capture_output=True,text=True).stdout.strip()
if version!='2.1.291 (Claude Code)':raise SystemExit('Client version differs from verified fixture version.')
cmd=[str(binary),'--print','--input-format','stream-json','--output-format','stream-json','--verbose','--restricted','--strict-mcp-config','--setting-sources','','--settings',str(settings),'--permission-mode','plan','--permission-prompts','host','--permission-prompt-tool','stdio','--tools','ExitPlanMode','--model','claude-sonnet-4-6']
p=subprocess.Popen(cmd,cwd=WORK,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
records=[]
def reader():
 for line in p.stdout:
  try:r=json.loads(line)
  except:continue
  records.append(r)
  if r.get('type')=='result':
   p.stdin.close()
  if r.get('type')=='control_request' and r.get('request',{}).get('subtype')=='can_use_tool':
   response={'type':'control_response','response':{'subtype':'success','request_id':r['request_id'],'response':{'behavior':'deny','message':'Controlled test: approval withheld.'}}}
   p.stdin.write(json.dumps(response)+'\n');p.stdin.flush()
threading.Thread(target=reader,daemon=True).start()
p.stdin.write(json.dumps({'type':'control_request','request_id':'fixture-init','request':{'subtype':'initialize','hooks':{}}})+'\n');p.stdin.flush()
p.stdin.write(json.dumps({'type':'user','message':{'role':'user','content':'Present the seeded plan using ExitPlanMode. This is a controlled test.'}})+'\n');p.stdin.flush()
try:p.wait(timeout=35)
except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=5)
server.shutdown();(ROOT/'client-output.json').write_text(json.dumps(records));(ROOT/'client-stderr.txt').write_text(p.stderr.read())
print(json.dumps({'version':version,'exit':p.returncode,'requests':len(requests),'hookObserved':(ROOT/'hook-input.json').exists(),'recordTypes':[r.get('type') for r in records]},ensure_ascii=False))

if p.returncode!=0 or not (ROOT/'hook-input.json').exists():raise SystemExit(1)
