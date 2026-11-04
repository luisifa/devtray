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
panel.navigate(3);panel.git_refresh_button.setEnabled(True);panel.git_refresh_button.setText('刷新')
panel.git_repos=[
 dict(project='atelier',projects=['atelier'],root='/tmp/demo/atelier',is_repo=True,error=None,branch='main',head='a1b2c3d4',upstream='origin/main',ahead=2,behind=1,detached=False,dirty=True,operation=None,warning=None,changes=[
  dict(path='api/src/main.rs',orig=None,code='M.',staged=True,unstaged=False,untracked=False,conflict=False),
  dict(path='web/src/App.vue',orig=None,code='.M',staged=False,unstaged=True,untracked=False,conflict=False),
  dict(path='docs/plan.md',orig='docs/old-plan.md',code='R.',staged=True,unstaged=False,untracked=False,conflict=False),
  dict(path='scratch/notes.txt',orig=None,code='??',staged=False,unstaged=True,untracked=True,conflict=False)]),
 dict(project='devtray',projects=['devtray'],root='/tmp/demo/devtray',is_repo=True,error=None,branch='codex/workspace',head='e5f6a7b8',upstream='origin/main',ahead=1,behind=0,detached=False,dirty=False,operation=None,warning=None,changes=[]),
 dict(project='notes',projects=['notes'],root='/tmp/demo/notes',is_repo=False,error='不是 Git 仓库',branch='',head=None,upstream=None,ahead=0,behind=0,detached=False,dirty=False,operation=None,warning=None,changes=[])]
panel.git_current='/tmp/demo/atelier';panel.git_loaded=True;panel.git_batch_summary='上次批量拉取：更新 2 个，已最新 1 个，跳过 1 个，失败 0 个'
panel.git_log=[
 dict(hash='a1b2c3d4e5',short='a1b2c3d4',author='ferychen',date='2026-11-03',subject='feat(git): 工作区仓库变更与批量拉取',decorations='HEAD -> main, origin/main'),
 dict(hash='b2c3d4e5f6',short='b2c3d4e5',author='ferychen',date='2026-11-02',subject='fix(panel): 保留资源选择与导航状态',decorations=''),
 dict(hash='c3d4e5f6a7',short='c3d4e5f6',author='ferychen',date='2026-11-01',subject='docs: 补充控制面板设计说明',decorations='tag: v0.1.1')]
panel.render_git_repos();panel.render_git_current();panel.render_git_log()
panel.git_message.setPlainText('feat(git): 增加工作区 Git 管理\n\n提交选中文件，拉取只做 fast-forward。')
panel.git_selected_path='api/src/main.rs';panel.git_diff_title.setText('改动预览 · api/src/main.rs')
panel.git_diff_view.setPlainText("""diff --git a/api/src/main.rs b/api/src/main.rs
index 3a3f9c1..8b21d4e 100644
--- a/api/src/main.rs
+++ b/api/src/main.rs
@@ -18,6 +18,9 @@ fn main() -> ExitCode {
     match args.first().map(String::as_str) {
         Some("status") => status(&args[1..]),
+        Some("git") => git::command(&args[1..]),
+        Some("_git-pull-start") => git::start_pull_command(&args[1..]),
+        Some("_git-pull-run") => git::run_pull_job(args.get(1)),
         Some("panel") => panel::command(&args[1..]),
         other => {
""")
panel.resize(1440,900);app.processEvents();panel.grab().save(str(output/'workspace-git.png'))
panel.resize(1040,720);app.processEvents();from PySide6.QtTest import QTest;QTest.qWait(60);app.processEvents();panel.grab().save(str(output/'workspace-git-1040.png'))
panel.resize(1440,900);panel.navigate(0);app.processEvents();panel.grab().save(str(output/'workspace-overview.png'))
panel.render_agents([dict(id='pi',name='Pi',bin='pi',path='/tmp/demo/bin/pi',version='1.4.0',provenance='native',package=None,update_command='pi update self',needs_sudo=False,notes=[]),dict(id='codex',name='Codex',bin='codex',path='/usr/bin/codex',version='0.165.0',provenance='pacman',package='openai-codex',update_command='sudo pacman -S openai-codex',needs_sudo=True,notes=[])])
panel.skills_inventory={'summary':dict(total=2,managed=1,unmanaged=1,drift_mirrors=1,update_available=1), 'skills':[
 dict(name='alpha',dir='/tmp/demo/skills/alpha',dir_exists=True,managed=True,source='owner/repo',tree_sha='a'*40,installed_at='2026-10-28',update='available',mirrors=[dict(agent='claude-code',path='/tmp/demo/claude/alpha',kind='copy',status='drift')],notes=[]),
 dict(name='local-workflow',dir='/tmp/demo/skills/local-workflow',dir_exists=True,managed=False,source=None,tree_sha=None,installed_at=None,update='unchecked',mirrors=[],notes=['来源未知'])]}
panel.render_skills();panel.navigate(4);app.processEvents();panel.grab().save(str(output/'workspace-agents.png'))
panel.job_timer.stop();panel.hide();panel.deleteLater();app.processEvents()
print('PASS: real Qt screenshots (fixture data), resources 1440/1040, git 1440/1040, overview, agents')
