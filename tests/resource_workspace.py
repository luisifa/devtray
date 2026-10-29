"""Qt regression for approved resource workspace; cleanup workers are stubbed, never real."""
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

TEMP = tempfile.TemporaryDirectory(prefix='devtray-workspace-')
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
os.environ['XDG_CONFIG_HOME'] = TEMP.name
os.environ['XDG_STATE_HOME'] = str(Path(TEMP.name)/'state')
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'ui'))
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication, QTreeWidgetItem
from panel import Panel
APP = QApplication.instance() or QApplication([])


def usage(name):
    return dict(project=name,root=f'/tmp/fixture/{name}',inferred=False,cleaning_allowed=True,complete=True,
        scanned_at=1,total_allocated=8192,total_logical=4096,
        artifacts=[dict(path='node_modules',kind='node_modules',allocated=8192,logical=4096,files=1,
                        deletable=True,evidence={'notes':[]})])


class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.config=Path(TEMP.name)/f'{self.id()}.toml'
        self.config.write_text('')
        self.panel=Panel(str(ROOT/'target/debug/devtray'), str(self.config), poll=False)
        self.w=self.panel
        self.w.render_services=lambda:None
        self.w.request=lambda *args,**kwargs:None
        self.w.data={'services':[{'project':name,'name':name+'/web','cwd':f'/tmp/fixture/{name}'} for name in ['alpha','beta','gamma']]}
        self.w.resource_usages={name:usage(name) for name in ['alpha','beta','gamma']}
        self.w.resource_selected='alpha'
        for name in ['alpha','beta','gamma']:
            item=QTreeWidgetItem([name]);item.setData(0,Qt.UserRole,('project',name))
            item.addChild(QTreeWidgetItem(['web']))
            self.w.tree.addTopLevelItem(item);item.setExpanded(True)
        self.w.render_resource_projects();self.w.render_resource_detail()

    def tearDown(self):
        self.w.job_timer.stop()
        self.w.hide();self.w.deleteLater();APP.processEvents()

    def choose(self,name):
        self.w.resource_selected=name;self.w.render_resource_detail()
        self.w.res_table.item(0,0).setCheckState(Qt.Checked)

    def request_handler(self,args,callback,**kwargs):
        self.calls.append(args)
        if args[0]=='_clean-plan':
            name=args[1];callback({'project':name,'total':8192,'candidates':[
                {'path':f'/tmp/fixture/{name}/node_modules','relative':'node_modules','allocated':8192,'kind':'node_modules'}]},None)
        elif args[0]=='_clean-start':
            callback({'job_id':args[1]+'-job','count':1},None)
        elif args[0]=='_disk-cache':callback([],None)
        elif args[0]=='_disk-scan':callback(usage(args[1]),None)

    def test_project_switch_and_sidebar_keep_choices(self):
        self.choose('alpha');self.choose('beta')
        self.w.select_tree(self.w.tree.topLevelItem(0))
        self.assertEqual(self.w.resource_selected,'alpha')
        self.assertEqual(self.w.res_table.item(0,0).checkState(),Qt.Checked)
        self.assertEqual(len(self.w.cleanup_selection.entries),2)
        self.assertEqual(self.w.res_selection_tree.topLevelItemCount(),2)
        self.w.load_agents_page=lambda:None
        self.w.navigate(3);self.w.show();APP.processEvents()
        self.assertTrue(self.w.tree.isVisible())
        self.assertEqual(len(self.w.cleanup_selection.entries),2)

    def test_search_restores_collapsed_projects(self):
        self.w.tree.topLevelItem(1).setExpanded(False)
        self.w.project_search.setText('web')
        self.assertTrue(self.w.tree.topLevelItem(1).isExpanded())
        self.w.project_search.clear()
        self.assertFalse(self.w.tree.topLevelItem(1).isExpanded())

    def test_new_root_and_partial_scan_invalidate_choices(self):
        self.choose('alpha');self.choose('beta')
        self.w.resource_usages['alpha']['root']='/tmp/fixture/moved'
        self.w.resource_selected='alpha';self.w.render_resource_detail()
        self.assertFalse(self.w.cleanup_selection.contains('alpha','node_modules'))
        self.assertTrue(self.w.cleanup_selection.contains('beta','node_modules'))
        self.w.resource_usages['alpha']['complete']=False
        self.w.render_resource_detail()
        self.w.res_table.item(0,0).setCheckState(Qt.Checked)
        self.assertFalse(self.w.cleanup_selection.contains('alpha','node_modules'))

    def test_remove_only_one_project_choice(self):
        self.choose('alpha');self.choose('beta')
        item=self.w.res_selection_tree.topLevelItem(0).child(0)
        self.w.res_selection_tree.setCurrentItem(item);self.w.remove_cleanup_selection()
        self.assertEqual(list(self.w.cleanup_selection.groups()),['beta'])

    def test_all_plans_before_confirmation_and_jobs_are_sequential(self):
        self.choose('alpha');self.choose('beta')
        self.calls=[];self.w.request=self.request_handler
        def confirm(plans):
            self.assertEqual([p['project'] for p in plans],['alpha','beta'])
            self.assertFalse(any(c[0]=='_clean-start' for c in self.calls))
            return True
        self.w.confirm_cleanup=confirm
        self.w.clean_selected()
        self.assertEqual(self.w.active_job,'alpha-job')
        self.assertEqual([c[1] for c in self.calls if c[0]=='_clean-start'],['alpha'])
        self.w.active_job=None
        self.w.finish_resource_job({'state':'done','result':{'removed':['/tmp/fixture/alpha/node_modules']}})
        self.assertEqual(self.w.active_job,'beta-job')
        self.assertFalse(self.w.cleanup_selection.contains('alpha','node_modules'))
        self.w.active_job=None
        self.w.finish_resource_job({'state':'done','result':{'removed':['/tmp/fixture/beta/node_modules']}})
        self.assertFalse(self.w.cleanup_busy)
        self.assertEqual(len(self.w.cleanup_selection.entries),0)

    def test_failed_preflight_and_cancel_never_start_workers(self):
        self.choose('alpha');self.choose('beta')
        self.calls=[]
        def refused(args,callback,**kwargs):
            self.calls.append(args);callback(None,'服务仍在运行')
        self.w.request=refused;self.w.clean_selected()
        self.assertFalse(self.w.cleanup_busy)
        self.assertFalse(any(c[0]=='_clean-start' for c in self.calls))
        self.calls=[];self.w.request=self.request_handler;self.w.confirm_cleanup=lambda _:False
        self.w.clean_selected()
        self.assertFalse(self.w.cleanup_busy)
        self.assertEqual(len(self.w.cleanup_selection.entries),2)
        self.assertFalse(any(c[0]=='_clean-start' for c in self.calls))

    def test_partial_failure_stops_remaining_and_keeps_selection(self):
        self.choose('alpha');self.choose('beta');self.choose('gamma')
        self.calls=[];self.w.request=self.request_handler;self.w.confirm_cleanup=lambda _:True
        self.w.clean_selected();self.w.active_job=None
        self.w.finish_resource_job({'state':'done','result':{'removed':[], 'failed':[{'path':'node_modules','error':'permission denied'}]}})
        self.assertFalse(self.w.cleanup_busy)
        self.assertEqual([c[1] for c in self.calls if c[0]=='_clean-start'],['alpha'])
        self.assertEqual(len(self.w.cleanup_selection.entries),3)

    def test_slow_poll_does_not_duplicate_job_transitions(self):
        self.w.active_job='slow';self.w.active_job_kind='clean'
        callbacks=[];self.w.request=lambda args,callback,**kwargs:callbacks.append(callback)
        self.w.poll_job();self.w.poll_job()
        self.assertEqual(len(callbacks),1)
        callbacks[0]({'state':'running','progress':.2},None)
        self.assertFalse(self.w.job_polling)

    def test_skill_context_menu_keeps_its_original_target_after_selection_changes(self):
        skill=dict(name='alpha',dir='/tmp/alpha',dir_exists=True,managed=True,mirrors=[])
        menu=self.w.make_skill_menu(skill)
        self.w.selected_skill_name=lambda:'beta'
        requests=[];self.w.request=lambda args,*a,**k:requests.append(args)
        next(a for a in menu.actions() if a.text()=='升级此技能').trigger()
        self.assertEqual(requests,[['_skills-plan','upgrade','alpha']])
        self.assertTrue(self.w.pending_write)
        self.w.clean_selected()
        self.assertEqual(requests,[['_skills-plan','upgrade','alpha']])

    def test_sidebar_and_three_panes_fit_minimum_window(self):
        self.w.request=lambda *a,**k:None
        self.w.pages.setCurrentIndex(2)
        self.w.resize(1040,720);self.w.show();APP.processEvents()
        self.assertEqual(self.w.width(),1040)
        self.assertTrue(self.w.tree.isVisible())
        self.assertEqual(self.w.resource_split.count(),3)
        for i in range(3):
            self.assertTrue(self.w.resource_split.widget(i).isVisible())
        self.assertGreater(self.w.tree.height(),80)
        self.assertTrue(self.w.res_clean_button.isVisible())

    def test_poll_error_keeps_worker_tracked_and_blocks_another_cleanup(self):
        self.choose('alpha')
        self.w.active_job='existing';self.w.active_job_kind='clean';self.w.cleanup_busy=True
        self.w.request=lambda args,callback,**kwargs:callback(None,'temporary read failure')
        self.w.poll_job()
        self.assertEqual(self.w.active_job,'existing')
        self.assertTrue(self.w.cleanup_busy)
        self.assertTrue(self.w.job_timer.isActive())

    def test_stopping_remaining_projects_keeps_unstarted_choices(self):
        self.choose('alpha');self.choose('beta')
        self.calls=[];self.w.request=self.request_handler;self.w.confirm_cleanup=lambda _:True
        self.w.clean_selected();self.w.stop_cleanup_queue()
        self.w.active_job=None
        self.w.finish_resource_job({'state':'done','result':{'removed':['/tmp/fixture/alpha/node_modules']}})
        self.assertEqual([c[1] for c in self.calls if c[0]=='_clean-start'],['alpha'])
        self.assertFalse(self.w.cleanup_busy)
        self.assertTrue(self.w.cleanup_selection.contains('beta','node_modules'))


if __name__=='__main__':unittest.main()
