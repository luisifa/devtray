"""Render the real Qt panel with isolated fixture data for visual review; no workers run."""
import os
from pathlib import Path
import sys
import tempfile

root=Path(__file__).resolve().parents[1]
temp=tempfile.TemporaryDirectory(prefix='devtray-render-')
os.environ.setdefault('QT_QPA_PLATFORM','offscreen')
os.environ['XDG_CONFIG_HOME']=temp.name
os.environ['XDG_STATE_HOME']=str(Path(temp.name)/'state')
sys.path.insert(0,str(root/'ui'))
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication
from panel import Panel
app=QApplication([])
config=Path(temp.name)/'config.toml';config.write_text('')
panel=Panel(str(root/'target/debug/devtray'),str(config),poll=False)
services=[]
for project,service,state,cmd in [('atelier','api','ready','cargo run'),('atelier','web','running','bun run dev'),('notes','worker','not_started','python worker.py'),('devtray','preview','not_started','cargo run')]:
 services.append(dict(project=project,service=service,name=f'{project}/{service}',cwd=f'/tmp/demo/{project}/{service}',cmd=cmd,state=state,pid=None,session=f'devtray-{project}-{service}',log=f'/tmp/demo/{project}/{service}.log',url='http://localhost:3000/',port=None,ready=None,autostart=False))
panel.request=lambda args,callback,**kwargs:None
panel.apply_data(dict(services=services,tmux_available=True,retention={'archives':10,'max_bytes':10485760}))
for name,size in [('atelier',93847552),('notes',45104562176),('devtray',13691752448)]:
 panel.resource_usages[name]=dict(project=name,root=f'/tmp/demo/{name}',inferred=False,cleaning_allowed=True,cleaning_reason=None,complete=True,scanned_at=1793210400,total_allocated=size,total_logical=size,
  artifacts=[dict(path='backend/target',kind='cargo_target',allocated=size,logical=size,files=500,deletable=True,evidence={'notes':['Cargo.toml 与构建痕迹已确认']}),
             dict(path='frontend/node_modules',kind='node_modules',allocated=109051904,logical=209715200,files=8300,deletable=True,evidence={'notes':['package.json 与 lockfile 已确认']}),
             dict(path='scripts/__pycache__',kind='py_cache',allocated=8192,logical=7000,files=3,deletable=True,evidence={'notes':[]}),
             dict(path='vendor',kind='extra',allocated=1024,logical=1024,files=1,deletable=False,evidence={'notes':['不在允许清理的清单']}),])
panel.resource_selected='atelier';panel.render_resource_projects();panel.render_resource_detail()
panel.res_table.item(0,0).setCheckState(Qt.Checked)
panel.resource_selected='notes';panel.render_resource_detail();panel.res_table.item(1,0).setCheckState(Qt.Checked)
panel.project=panel.resource_selected='atelier';panel.sync_project_tree();panel.render_resource_projects();panel.render_resource_detail()
output=root/'docs/screenshots';output.mkdir(parents=True,exist_ok=True)
panel.resize(1440,900);panel.show();app.processEvents();panel.navigate(2);app.processEvents()
panel.grab().save(str(output/'workspace-resources.png'))
panel.resize(1040,720);app.processEvents();panel.grab().save(str(output/'workspace-resources-1040.png'))
panel.resize(1440,900);panel.navigate(0);app.processEvents();panel.grab().save(str(output/'workspace-overview.png'))
panel.render_agents([dict(id='pi',name='Pi',bin='pi',path='/tmp/demo/bin/pi',version='1.4.0',provenance='native',package=None,update_command='pi update self',needs_sudo=False,notes=[]),dict(id='codex',name='Codex',bin='codex',path='/usr/bin/codex',version='0.165.0',provenance='pacman',package='openai-codex',update_command='sudo pacman -S openai-codex',needs_sudo=True,notes=[])])
panel.skills_inventory={'summary':dict(total=2,managed=1,unmanaged=1,drift_mirrors=1,update_available=1), 'skills':[
 dict(name='alpha',dir='/tmp/demo/skills/alpha',dir_exists=True,managed=True,source='owner/repo',tree_sha='a'*40,installed_at='2026-10-28',update='available',mirrors=[dict(agent='claude-code',path='/tmp/demo/claude/alpha',kind='copy',status='drift')],notes=[]),
 dict(name='local-workflow',dir='/tmp/demo/skills/local-workflow',dir_exists=True,managed=False,source=None,tree_sha=None,installed_at=None,update='unchecked',mirrors=[],notes=['来源未知'])]}
panel.render_skills();panel.navigate(3);app.processEvents();panel.grab().save(str(output/'workspace-agents.png'))
panel.job_timer.stop();panel.hide();panel.deleteLater();app.processEvents()
print('PASS: real Qt screenshots (fixture data), resources 1440/1040, overview, agents')
