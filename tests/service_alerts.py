"""Failed-service acknowledgement through real CLI + Qt, with a fake tmux snapshot."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

os.environ.setdefault('QT_QPA_PLATFORM','offscreen')
ROOT=Path(__file__).resolve().parents[1]
BINARY=os.environ.get('DEVTRAY_TEST_BINARY',str(ROOT/'target/debug/devtray'))
TEMP=tempfile.TemporaryDirectory(prefix='devtray-alert-tests-')
os.environ['XDG_CONFIG_HOME']=TEMP.name
sys.path.insert(0,str(ROOT/'ui'))
from PySide6.QtWidgets import QApplication, QPushButton
from PySide6.QtCore import Qt
from PySide6.QtGui import QTextCursor
from panel import Panel
APP=QApplication.instance() or QApplication([])


def wait_for(condition):
    end=time.monotonic()+5
    while not condition() and time.monotonic()<end:
        APP.processEvents();time.sleep(.01)
    assert condition(), 'Qt state did not update'


class AlertTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(dir=TEMP.name)
        self.root=Path(self.tmp.name)
        self.config=self.root/'config.toml'
        self.log=self.root/'web.log'
        self.log.write_text('ERROR web failure\nINFO retained log\n')
        self.config.write_text(f'[[project]]\nname="hb_ecom"\n[[project.service]]\nname="web"\ncwd="{self.root}"\ncmd="false"\nlog_file="{self.log}"\n')
        self.snapshot=self.root/'snapshot'
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t4242\n')
        bindir=self.root/'bin';bindir.mkdir()
        tmux=bindir/'tmux';tmux.write_text('#!/bin/sh\ncat "$DEVTRAY_TEST_SNAPSHOT"\n');tmux.chmod(0o755)
        self.oldenv={k:os.environ.get(k) for k in ['PATH','XDG_STATE_HOME','DEVTRAY_TEST_SNAPSHOT','DEVTRAY_CONFIG']}
        os.environ.update(PATH=str(bindir)+':'+os.environ['PATH'],XDG_STATE_HOME=str(self.root/'state'),DEVTRAY_TEST_SNAPSHOT=str(self.snapshot),DEVTRAY_CONFIG=str(self.config))
        self.w=Panel(BINARY,str(self.config),poll=False)
        self.w.show();self.w.refresh()
        wait_for(lambda:not self.w.refreshing and bool(self.w.data['services']))

    def tearDown(self):
        for process in list(self.w.processes):process.kill();process.waitForFinished(1000)
        self.w.hide();self.w.deleteLater();APP.processEvents()
        for key,value in self.oldenv.items():
            if value is None:os.environ.pop(key,None)
            else:os.environ[key]=value
        self.tmp.cleanup()

    def test_failed_service_has_acknowledgement_without_a_banner(self):
        self.assertTrue(self.w.banner.isHidden())
        wait_for(lambda:any(b.text()=='标记已读' and b.isVisible() for b in self.w.findChildren(QPushButton)))
        buttons=[b for b in self.w.findChildren(QPushButton) if b.text()=='标记已读' and b.isVisible()]
        self.assertGreaterEqual(len(buttons),1,'failed service has no visible acknowledgement action')
        screenshot_dir=os.environ.get('DEVTRAY_ALERT_SCREENSHOTS')
        if screenshot_dir:
            Path(screenshot_dir).mkdir(parents=True,exist_ok=True)
            self.w.grab().save(str(Path(screenshot_dir)/'service-alert-unread.png'))
        buttons[0].click()
        wait_for(lambda:self.w.data['services'][0].get('alert_acknowledged') is True)
        service=self.w.data['services'][0]
        self.assertEqual(service['state'],'exited_failed')
        self.assertEqual(service['exit_code'],1)
        self.assertEqual(self.w.metrics[2].text(),'00')
        if screenshot_dir:
            APP.processEvents()
            self.w.grab().save(str(Path(screenshot_dir)/'service-alert-read.png'))
        # A separate process must see the same acknowledgement, without a tray signal.
        data=json.loads(subprocess.check_output([BINARY,'_panel-data'],text=True))
        self.assertTrue(data['services'][0]['alert_acknowledged'])
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t5252\n')
        self.w.refresh()
        wait_for(lambda:not self.w.refreshing)
        self.assertFalse(self.w.data['services'][0]['alert_acknowledged'])

    def test_small_panel_also_exposes_read_action_and_updates_badge(self):
        self.w.present('small')
        menu=self.w.quick_menu
        wait_for(lambda:menu.isVisible())
        row=menu.projects['hb_ecom'].rows['hb_ecom/web']
        wait_for(lambda:row.acknowledge.isVisible())
        row.acknowledge.click()
        wait_for(lambda:self.w.data['services'][0].get('alert_acknowledged') is True)
        self.assertIn('已读',row.state.text())
        self.assertTrue(row.acknowledge.isHidden())
        self.assertEqual(self.w.metrics[2].text(),'00')
        menu.hide();menu.timer.stop()

    def test_stale_read_request_does_not_acknowledge_a_new_failure(self):
        token=self.w.data['services'][0]['failure_token']
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t5252\n')
        result=subprocess.run([BINARY,'ack','hb_ecom/web','--failure-token',token],capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        data=json.loads(subprocess.check_output([BINARY,'_panel-data'],text=True))
        self.assertFalse(data['services'][0]['alert_acknowledged'])

    def test_acknowledging_one_service_leaves_other_failure_unread(self):
        self.config.write_text(self.config.read_text()+f'[[project.service]]\nname="api"\ncwd="{self.root}"\ncmd="false"\n')
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t4242\ndevtray-hb_ecom-api\t1\t2\t4243\n')
        subprocess.run([BINARY,'ack','hb_ecom/web'],capture_output=True,text=True,check=True)
        data=json.loads(subprocess.check_output([BINARY,'_panel-data'],text=True))
        values={service['name']:service['alert_acknowledged'] for service in data['services']}
        self.assertEqual(values,{'hb_ecom/web':True,'hb_ecom/api':False})

    def add_other_failures(self):
        self.api_log=self.root/'api.log'
        self.api_log.write_text('ERROR api failure\n')
        self.config.write_text(self.config.read_text()+f'[[project.service]]\nname="api"\ncwd="{self.root}"\ncmd="false"\nlog_file="{self.api_log}"\n[[project]]\nname="other"\n[[project.service]]\nname="worker"\ncwd="{self.root}"\ncmd="false"\n')
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t4242\ndevtray-hb_ecom-api\t1\t2\t4243\ndevtray-other-worker\t1\t3\t4244\n')
        self.w.refresh()
        wait_for(lambda:not self.w.refreshing and len(self.w.data['services'])==3)

    def alert_row(self, name):
        for index in range(self.w.alert_table.rowCount()):
            if self.w.alert_table.item(index,0).data(Qt.UserRole)==name:
                return index
        self.fail('Missing exception row: '+name)

    def alert_action(self, name, text):
        row=self.alert_row(name)
        return next(b for b in self.w.alert_table.cellWidget(row,4).findChildren(QPushButton) if b.text()==text)

    def test_project_button_acknowledges_only_that_projects_current_failures(self):
        self.add_other_failures()
        project=self.w.tree.topLevelItem(1)
        self.w.select_tree(project)
        self.w.navigate(1)
        wait_for(lambda:any(b.isVisible() and b.accessibleName()=='项目异常标记已读 hb_ecom' for b in self.w.findChildren(QPushButton)))
        control=next(b for b in self.w.findChildren(QPushButton) if b.isVisible() and b.accessibleName()=='项目异常标记已读 hb_ecom')
        control.click()
        wait_for(lambda:not self.w.action_pending and sum(bool(s.get('alert_acknowledged')) for s in self.w.data['services'])==2)
        by_name={s['name']:s for s in self.w.data['services']}
        self.assertFalse(by_name['other/worker']['alert_acknowledged'])
        self.assertEqual(by_name['hb_ecom/api']['exit_code'],2)
        self.assertEqual(self.w.alert_project.currentData(),'hb_ecom')
        self.assertEqual(self.w.alert_table.rowCount(),2)

    def test_log_exception_row_reads_one_failure_and_preserves_log_selection(self):
        self.add_other_failures()
        self.w.navigate(4)
        wait_for(lambda:not self.w.log_loading and bool(self.w.log_view.toPlainText()))
        self.assertEqual(self.w.alert_table.rowCount(),3)
        self.alert_action('hb_ecom/api','查看日志').click()
        wait_for(lambda:not self.w.log_loading and 'api failure' in self.w.log_view.toPlainText())
        self.assertEqual(self.w.log_service.currentData(),'hb_ecom/api')
        self.w.freeze_logs()
        cursor=self.w.log_view.textCursor();cursor.setPosition(0);cursor.setPosition(9,QTextCursor.KeepAnchor)
        self.w.log_view.setTextCursor(cursor)
        selection=cursor.selectedText()
        self.alert_action('hb_ecom/api','标记已读').click()
        wait_for(lambda:any(s['name']=='hb_ecom/api' and s.get('alert_acknowledged') for s in self.w.data['services']))
        self.assertEqual(self.w.alert_table.item(self.alert_row('hb_ecom/api'),3).text(),'已读')
        self.assertFalse(self.alert_action('hb_ecom/api','已读').isEnabled())
        self.assertEqual(self.w.log_view.textCursor().selectedText(),selection)
        self.assertTrue(self.w.paused)
        self.assertEqual(self.w.alert_count.text(),'2 未读 / 1 已读')
        self.w.alert_status.setCurrentIndex(self.w.alert_status.findData('unread'))
        self.assertEqual(self.w.alert_table.rowCount(),2)
        self.w.alert_status.setCurrentIndex(self.w.alert_status.findData('read'))
        self.assertEqual(self.w.alert_table.rowCount(),1)
        self.assertEqual(self.w.alert_table.item(0,1).text(),'api')

    def test_new_failure_reappears_unread_in_log_list_and_buttons_report_stale_ack(self):
        self.w.navigate(4)
        row=self.alert_row('hb_ecom/web')
        control=self.alert_action('hb_ecom/web','标记已读')
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t5252\n')
        control.click()
        wait_for(lambda:not self.w.action_pending and not self.w.refreshing)
        self.assertFalse(self.w.data['services'][0]['alert_acknowledged'])
        self.assertIn('新的异常',self.w.banner.text())
        self.assertTrue(self.alert_action('hb_ecom/web','标记已读').isEnabled())
        self.alert_action('hb_ecom/web','标记已读').click()
        wait_for(lambda:self.w.data['services'][0].get('alert_acknowledged'))
        self.snapshot.write_text('devtray-hb_ecom-web\t1\t1\t6262\n')
        self.w.refresh()
        wait_for(lambda:not self.w.refreshing)
        self.assertEqual(self.w.alert_table.item(self.alert_row('hb_ecom/web'),3).text(),'未读')

    def test_empty_queue_and_sidebar_project_switch(self):
        self.add_other_failures()
        self.w.navigate(4)
        other=self.w.tree.topLevelItem(2)
        self.w.select_tree(other)
        self.assertEqual(self.w.alert_project.currentData(),'other')
        self.assertEqual(self.w.alert_table.rowCount(),1)
        self.assertEqual(next(s for s in self.w.data['services'] if s['name']==self.w.selected)['project'],'other')
        self.snapshot.write_text('')
        self.w.refresh()
        wait_for(lambda:not self.w.refreshing)
        self.assertEqual(self.w.alert_table.rowCount(),0)
        self.assertTrue(self.w.alert_empty.isVisible())
        self.assertTrue(self.w.alert_table.isHidden())

    def test_ack_refreshes_after_an_older_status_request_finishes(self):
        callbacks=[]
        self.w.request=lambda args,callback,*a,**kw:callbacks.append((args,callback))
        self.w.refresh()
        old=self.w.data
        self.w.acknowledge_service(old['services'][0])
        next(callback for args,callback in callbacks if args[0]=='ack')({},None)
        next(callback for args,callback in callbacks if args[0]=='_panel-data')(old,None)
        APP.processEvents()
        self.assertEqual(sum(args[0]=='_panel-data' for args,callback in callbacks),2)


if __name__=='__main__':unittest.main()
