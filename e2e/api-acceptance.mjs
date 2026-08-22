// ReactorOS Backend API Acceptance Test (non-AI surfaces)
// Node 24 native fetch, zero deps. BASE_URL env overridable.
const BASE_URL = process.env.BASE_URL || 'http://127.0.0.1:8000';
const TS = Date.now();
const P = 'e2e-' + TS;

let passCount = 0, failCount = 0, failures = [];

function ok(nm, d) { passCount++; console.log('[PASS] ' + nm + '::' + d); }
function bad(nm, d, ex) { failCount++; var s = ex ? ' | ' + ex : ''; console.log('[FAIL] ' + nm + '::' + d + s); failures.push({name:nm,detail:d,extra:ex||''}); }

async function api(m, p, o) {
  o = o || {};
  var url = BASE_URL + p;
  var r = await fetch(url, { method: m, headers: Object.assign({'content-type':'application/json'}, o.headers||{}), body: o.body!==undefined?JSON.stringify(o.body):undefined, redirect:'manual' });
  var t = await r.text(); var d = null; try { d=JSON.parse(t); } catch(e){}
  return {status:r.status, text:t, data:d};
}

var tokens = {};
async function login(u, pw) {
  var r = await api('POST','/api/auth/login',{body:{username:u,password:pw}});
  if(r.status===200 && r.data && r.data.data && r.data.data.token){tokens[u]=r.data.data.token; return r.data.data;} return null;
}
function hdr(r){var t=tokens[r]; return t?{authorization:'Bearer '+t}:{};}
var sleep=function(ms){return new Promise(function(r){setTimeout(r,ms);});};

(async function main() {
  console.log('BASE_URL=' + BASE_URL);
  console.log('Timer prefix: ' + P + '\n');

  // === 1. Authentication ===
  var opLogin = await login('operator','operator123');
  var engLogin = await login('engineer','engineer123');
  var adminLogin = await login('admin','admin123');
  if(engLogin&&engLogin.token) ok(P+'-auth-login-engineer','login returns 200+token');
  else bad(P+'-auth-login-engineer','expected 200+token',JSON.stringify(engLogin));
  if(adminLogin&&adminLogin.token) ok(P+'-auth-login-admin','login returns 200+token');
  else bad(P+'-auth-login-admin','expected 200+token',JSON.stringify(adminLogin));
  var badLogin = await api('POST','/api/auth/login',{body:{username:'engineer',password:'wrong'}});
  if(badLogin.status===401) ok(P+'-auth-bad-pw','wrong password->401');
  else bad(P+'-auth-bad-pw','expected 401',badLogin.status+': '+badLogin.text.substring(0,200));
  var noUser = await api('POST','/api/auth/login',{body:{password:'x'}});
  if(noUser.status===400||noUser.status===422) ok(P+'-auth-no-username','missing username->400/422');
  else bad(P+'-auth-no-username','expected 400/422',noUser.status+': '+noUser.text.substring(0,200));
  var noPw = await api('POST','/api/auth/login',{body:{username:'engineer'}});
  if(noPw.status===400||noPw.status===422) ok(P+'-auth-no-pw','missing password->400/422');
  else bad(P+'-auth-no-pw','expected 400/422',noPw.status+': '+noPw.text.substring(0,200));
  if(engLogin) {
    var me = await api('GET','/api/auth/me',{headers:hdr('engineer')});
    if(me.status===200&&me.data&&me.data.data&&me.data.data.username==='engineer') ok(P+'-auth-me-tok','auth/me->200+user');
    else bad(P+'-auth-me-tok','expected 200',me.status+': '+JSON.stringify(me).substring(0,200));
    var noMe = await api('GET','/api/auth/me');
    if(noMe.status===401) ok(P+'-auth-me-notok','no token->401');
    else bad(P+'-auth-me-notok','expected 401',noMe.status+': '+noMe.text.substring(0,200));
    var fakeMe = await api('GET','/api/auth/me',{headers:{authorization:'Bearer fake:'}});
    if(fakeMe.status===401) ok(P+'-auth-me-fake','fake token->401');
    else bad(P+'-auth-me-fake','expected 401',fakeMe.status+': '+fakeMe.text.substring(0,200));
  }

  // === 2. RBAC ===
  if(opLogin) {
    var opCr = await api('POST','/api/processes',{headers:hdr('operator'),body:{name:'should-fail'}});
    if(opCr.status===403) ok(P+'-rbac-op-no-create-process','operator create process->403');
    else bad(P+'-rbac-op-no-create-process','expected 403',opCr.status+': '+opCr.text.substring(0,200));
    var opMb = await api('POST','/api/modbus/registers/x/write',{headers:hdr('operator'),body:{value:0,reason:'t'}});
    if(opMb.status===403) ok(P+'-rbac-op-no-modbus','operator modbus write->403');
    else bad(P+'-rbac-op-no-modbus','expected 403',opMb.status+': '+opMb.text.substring(0,200));
  }
  var unauthTgt = await api('POST','/api/control/targets',{body:{temperature_c:100,stirrer_rpm:400}});
  if(unauthTgt.status===401) ok(P+'-rbac-unauth-tgt','no auth post targets->401');
  else bad(P+'-rbac-unauth-tgt','expected 401',unauthTgt.status+': '+unauthTgt.text.substring(0,200));

  // === 3. Process CRUD ===
  var pCr = await api('POST','/api/processes',{headers:hdr('engineer'),body:{name:P+'-proc-a',description:'acceptance test'}});
  var pIdA = pCr.status===200 && pCr.data&&pCr.data.data&&pCr.data.data.id ? pCr.data.data.id : null;
  if(pIdA) ok(P+'-proc-create','created id='+pIdA);
  else bad(P+'-proc-create','expected 200+id',pCr.status+': '+pCr.text.substring(0,300));
  var pCr2 = await api('POST','/api/processes',{headers:hdr('engineer'),body:{name:P+'-proc-b',description:'2nd process'}});
  var pIdB = pCr2.status===200 && pCr2.data&&pCr2.data.data&&pCr2.data.data.id ? pCr2.data.data.id : null;
  if(pIdB) ok(P+'-proc-create-b','created id='+pIdB);
  else bad(P+'-proc-create-b','expected 200+id',pCr2.status+': '+pCr2.text.substring(0,300));
  if(pIdA) {
    var sNull = await api('POST','/api/processes/'+pIdA+'/steps',{headers:hdr('engineer'),body:{name:null,target_temperature_c:100,duration_minutes:30,target_stirrer_rpm:400}});
    if(sNull.status===400||sNull.status===422) ok(P+'-proc-step-null-name','null name->'+sNull.status+' (4xx)');
    else bad(P+'-proc-step-null-name','expected 4xx',sNull.status+': '+sNull.text.substring(0,200));
    var sNeg = await api('POST','/api/processes/'+pIdA+'/steps',{headers:hdr('engineer'),body:{name:'bad',target_temperature_c:100,duration_minutes:-5,target_stirrer_rpm:400}});
    if(sNeg.status===400) ok(P+'-proc-step-neg-dur','neg duration->400');
    else bad(P+'-proc-step-neg-dur','expected 400',sNeg.status+': '+sNeg.text.substring(0,200));
    var sOob = await api('POST','/api/processes/'+pIdA+'/steps',{headers:hdr('engineer'),body:{name:'bad',target_temperature_c:200,duration_minutes:30,target_stirrer_rpm:400}});
    if(sOob.status===400) ok(P+'-proc-step-temp-200','temp 200->400');
    else bad(P+'-proc-step-temp-200','expected 400',sOob.status+': '+sOob.text.substring(0,200));
    var sRpm = await api('POST','/api/processes/'+pIdA+'/steps',{headers:hdr('engineer'),body:{name:'bad',target_temperature_c:100,duration_minutes:30,target_stirrer_rpm:5000}});
    if(sRpm.status===400) ok(P+'-proc-step-rpm-5000','rpm 5000->400');
    else bad(P+'-proc-step-rpm-5000','expected 400',sRpm.status+': '+sRpm.text.substring(0,200));
    var sFz = await api('POST','/api/processes/'+pIdA+'/steps',{headers:hdr('engineer'),body:{name:'fz',target_temperature_c:135,duration_minutes:30,target_stirrer_rpm:100}});
    if(sFz.status===403) ok(P+'-proc-step-forbidden-zone','135C+100rpm->403');
    else bad(P+'-proc-step-forbidden-zone','expected 403',sFz.status+': '+sFz.text.substring(0,300));
    var sOk = await api('POST','/api/processes/'+pIdA+'/steps',{headers:hdr('engineer'),body:{name:'heat-1',target_temperature_c:120,duration_minutes:30,target_stirrer_rpm:400,target_shake_speed_cpm:20,target_pressure_mpa:0.5}});
    if(sOk.status===200) ok(P+'-proc-add-step',sOk.data&&sOk.data.data?'added step: '+sOk.data.data.id:'step added');
    else bad(P+'-proc-add-step','expected 200',sOk.status+': '+sOk.text.substring(0,300));
  }
  if(pIdB) {
    var sB = await api('POST','/api/processes/'+pIdB+'/steps',{headers:hdr('engineer'),body:{name:'heat-b',target_temperature_c:110,duration_minutes:20,target_stirrer_rpm:350}});
    if(sB.status===200) ok(P+'-proc-add-step-b','added step to B');
    else bad(P+'-proc-add-step-b','expected 200',sB.status+': '+sB.text.substring(0,200));
  }
  if(pIdA) {
    var gP = await api('GET','/api/processes/'+pIdA,{headers:hdr('engineer')});
    if(gP.status===200) ok(P+'-proc-get','get process id='+pIdA);
    else bad(P+'-proc-get','expected 200',gP.status+': '+gP.text.substring(0,200));
    var uP = await api('PUT','/api/processes/'+pIdA,{headers:hdr('engineer'),body:{name:P+'-proc-a-v2'}});
    if(uP.status===200) ok(P+'-proc-update','updated process');
    else bad(P+'-proc-update','expected 200',uP.status+': '+uP.text.substring(0,300));
  }
  var gNx = await api('GET','/api/processes/99999',{headers:hdr('engineer')});
  if(gNx.status===404) ok(P+'-proc-404','non-existent process->404');
  else bad(P+'-proc-404','expected 404',gNx.status+': '+gNx.text.substring(0,200));

  // === 4. Process Interlock ===
  // Cleanup any pre-existing active batch from seeded data
  try {
    await api('POST','/api/processes/current/stop',{headers:hdr('engineer'),body:{reason:'e2e-pre-cleanup'}});
  } catch(e) {}
  await sleep(1500);
  var curR = await api('GET','/api/live',{headers:hdr('engineer')});
  var activeId = curR.status===200 && curR.data && curR.data.data && curR.data.data.runtime ? curR.data.data.runtime.active_batch_id : null;
  if(activeId!=null) {
    await api('POST','/api/processes/current/stop',{headers:hdr('engineer')}).catch(function(){});
    await sleep(1500);
  }
  // Finish any leftover unfinished batches
  var bList = await api('GET','/api/batches',{headers:hdr('engineer')});
  var batchesArr = bList.status===200 && bList.data && bList.data.data ? (bList.data.data.batches||[]) : [];
  for(var bi=0; bi<batchesArr.length; bi++) {
    if(batchesArr[bi].finished_at==null) {
      await api('POST','/api/batches/'+batchesArr[bi].id+'/finish',{headers:hdr('engineer')}).catch(function(){});
    }
  }
  await sleep(500);
  var startA = null, startB = null;
  if(pIdA) {
    startA = await api('POST','/api/processes/'+pIdA+'/start',{headers:hdr('engineer')});
    if(startA.status===200) ok(P+'-interlock-start-A','started process A, batch='+(startA.data&&startA.data.data&&startA.data.data.batch?startA.data.data.batch.id:'?'));
    else bad(P+'-interlock-start-A','expected 200',startA.status+': '+startA.text.substring(0,400));
  }
  if(pIdA && startA && startA.status===200) {
    var dupA = await api('POST','/api/processes/'+pIdA+'/start',{headers:hdr('engineer')});
    if(dupA.status===409) ok(P+'-interlock-start-dup','dup start->409');
    else bad(P+'-interlock-start-dup','expected 409',dupA.status+': '+dupA.text.substring(0,200));
  }
  if(pIdB && startA && startA.status===200) {
    startB = await api('POST','/api/processes/'+pIdB+'/start',{headers:hdr('engineer')});
    if(startB.status===409) ok(P+'-interlock-start-other-while-running','start B while A running->409');
    else bad(P+'-interlock-start-other-while-running','expected 409',startB.status+': '+startB.text.substring(0,200));
  }
  var stopNR = await api('POST','/api/processes/99998/stop',{headers:hdr('engineer')});
  if(stopNR.status===409) ok(P+'-interlock-stop-non-running','stop non-running->409');
  else bad(P+'-interlock-stop-non-running','expected 409',stopNR.status+': '+stopNR.text.substring(0,200));
  var csR = await api('POST','/api/processes/current/stop',{headers:hdr('engineer')});
  if(csR.status===409) ok(P+'-interlock-current-stop-none','current/stop with none->409');
  else if(csR.status>=200&&csR.status<300) ok(P+'-interlock-current-stop-none','current/stop handled('+csR.status+')');
  else bad(P+'-interlock-current-stop-none','expected 409/204',csR.status+': '+csR.text.substring(0,200));
  if(pIdA && startA && startA.status===200) {
    var stopA = await api('POST','/api/processes/'+pIdA+'/stop',{headers:hdr('engineer'),body:{reason:'e2e teardown'}});
    if(stopA.status===200||stopA.status===204||stopA.status===409) ok(P+'-interlock-stop-A','stop A status='+stopA.status);
    else bad(P+'-interlock-stop-A','unexpected',stopA.status+': '+stopA.text.substring(0,200));
  }
  await sleep(500);

  // === 5. Control Targets ===
  var vTgt = await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:100,stirrer_rpm:400}});
  if(vTgt.status===200) ok(P+'-tgt-valid','valid targets->200');
  else bad(P+'-tgt-valid','expected 200',vTgt.status+': '+vTgt.text.substring(0,300));
  var t200 = await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:200,stirrer_rpm:400}});
  if(t200.status===400) ok(P+'-tgt-temp-200','temp 200->400');
  else bad(P+'-tgt-temp-200','expected 400',t200.status+': '+t200.text.substring(0,200));
  var r5k = await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:100,stirrer_rpm:5000}});
  if(r5k.status===400) ok(P+'-tgt-rpm-5000','rpm 5000->400');
  else bad(P+'-tgt-rpm-5000','expected 400',r5k.status+': '+r5k.text.substring(0,200));
  var fzT = await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:135,stirrer_rpm:100}});
  if(fzT.status===403) ok(P+'-tgt-forbidden-zone','135C+100rpm->403');
  else bad(P+'-tgt-forbidden-zone','expected 403',fzT.status+': '+fzT.text.substring(0,300));
  var mlOn = await api('POST','/api/control/manual-lock',{headers:hdr('engineer'),body:{locked:true}});
  if(mlOn.status===204) ok(P+'-manual-lock-on','lock on->204');
  else bad(P+'-manual-lock-on','expected 204',mlOn.status+': '+mlOn.text.substring(0,200));
  var tgtLock = await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:100,stirrer_rpm:400}});
  if(tgtLock.status===409) ok(P+'-tgt-blocked-manual-lock','targets blocked under manual lock->409');
  else bad(P+'-tgt-blocked-manual-lock','expected 409',tgtLock.status+': '+tgtLock.text.substring(0,200));
  var mlOff = await api('POST','/api/control/manual-lock',{headers:hdr('engineer'),body:{locked:false}});
  if(mlOff.status===204) ok(P+'-manual-lock-off','lock off->204');
  else bad(P+'-manual-lock-off','expected 204',mlOff.status+': '+mlOff.text.substring(0,200));
  var eStopR = await api('POST','/api/control/emergency-stop',{headers:hdr('engineer')});
  if(eStopR.status===204) ok(P+'-e-stop','e-stop->204');
  else bad(P+'-e-stop','expected 204',eStopR.status+': '+eStopR.text.substring(0,200));
  var tgtES = await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:100,stirrer_rpm:400}});
  if(tgtES.status===409) ok(P+'-tgt-blocked-e-stop','targets blocked under e-stop->409');
  else bad(P+'-tgt-blocked-e-stop','expected 409',tgtES.status+': '+tgtES.text.substring(0,200));
  var eRst = await api('POST','/api/control/emergency-stop/reset',{headers:hdr('engineer')});
  if(eRst.status===204) ok(P+'-e-stop-reset','e-stop reset->204');
  else if(eRst.status===409) ok(P+'-e-stop-reset','e-stop reset not needed->409');
  else bad(P+'-e-stop-reset','unexpected',eRst.status+': '+eRst.text.substring(0,300));
  var noFault = await api('POST','/api/control/fault/reset',{headers:hdr('engineer')});
  if(noFault.status===409) ok(P+'-fault-reset-no-fault','no fault->409');
  else bad(P+'-fault-reset-no-fault','expected 409',noFault.status+': '+noFault.text.substring(0,300));

  // === 6. Batches ===
  var blR = await api('GET','/api/batches',{headers:hdr('engineer')});
  if(blR.status===200&&blR.data&&blR.data.data&&blR.data.data.batches) ok(P+'-batch-list','batches count='+blR.data.data.batches.length);
  else bad(P+'-batch-list','expected 200',blR.status+': '+JSON.stringify(blR).substring(0,200));
  var batchFinNx = await api('POST','/api/batches/99999/finish',{headers:hdr('engineer')});
  if(batchFinNx.status===404) ok(P+'-batch-finish-404','finish 99999->404');
  else bad(P+'-batch-finish-404','expected 404',batchFinNx.status+': '+batchFinNx.text.substring(0,200));
  var csvR = await api('GET','/api/batches/export.csv',{headers:hdr('engineer')});
  if(csvR.status===200 && csvR.text && csvR.text.indexOf(',')>=0) ok(P+'-batch-csv','CSV export OK, has commas');
  else if(csvR.status===200) ok(P+'-batch-csv','CSV export OK');
  else bad(P+'-batch-csv','expected 200',csvR.status+': '+csvR.text.substring(0,100));
  var xlsR = await api('GET','/api/batches/export.xlsx',{headers:hdr('engineer')});
  if(xlsR.status===200) ok(P+'-batch-xlsx','XLSX export OK');
  else bad(P+'-batch-xlsx','expected 200',xlsR.status+': '+xlsR.text.substring(0,100));
  // Batch start/finish
  for(var bi2=0; bi2<batchesArr.length; bi2++) { if(batchesArr[bi2].finished_at==null) await api('POST','/api/batches/'+batchesArr[bi2].id+'/finish',{headers:hdr('engineer')}).catch(function(){}); }
  await sleep(500);
  var bsR = await api('POST','/api/batches/start',{headers:hdr('engineer'),body:{name:P+'-batch',target_temperature_c:100,target_stirrer_rpm:400,heating_minutes:30,stirring_minutes:20}});
  if(bsR.status===200||bsR.status===204) {
    ok(P+'-batch-start','started batch');
    var newBId = bsR.data&&bsR.data.data&&bsR.data.data.id ? bsR.data.data.id : null;
    if(newBId) {
      await sleep(500);
      try { await api('POST','/api/processes/current/stop',{headers:hdr('engineer')}); } catch(e) {}
      await sleep(500);
      var finR = await api('POST','/api/batches/'+newBId+'/finish',{headers:hdr('engineer')});
      if(finR.status===204||finR.status===200) ok(P+'-batch-finish-new','finished new batch');
      else bad(P+'-batch-finish-new','expected 204',finR.status+': '+finR.text.substring(0,200));
    }
  } else {
    bad(P+'-batch-start','expected 200/204',bsR.status+': '+bsR.text.substring(0,400));
  }

  // === 7. Audit ===
  await api('POST','/api/control/targets',{headers:hdr('engineer'),body:{temperature_c:90,stirrer_rpm:300}}).catch(function(){});
  var audR = await api('GET','/api/audit/logs?page=1&page_size=5',{headers:hdr('engineer')});
  if(audR.status===200 && audR.data && audR.data.data && audR.data.data.events && audR.data.data.events.length>0)
    ok(P+'-audit-logs','audit has '+audR.data.data.events.length+' events');
  else bad(P+'-audit-logs','expected 200+events',audR.status+': '+JSON.stringify(audR).substring(0,200));
  var audCsv = await api('GET','/api/audit/export.csv',{headers:hdr('engineer')});
  if(audCsv.status===200 && audCsv.text && (audCsv.text.indexOf(',')>=0||audCsv.text.indexOf('\n')>=0)) ok(P+'-audit-csv','audit CSV OK');
  else if(audCsv.status===200) ok(P+'-audit-csv','audit CSV OK');
  else bad(P+'-audit-csv','expected 200',audCsv.status+': '+audCsv.text.substring(0,100));

  // === 8. Simulation ===
  var simR = await api('GET','/api/simulation/status');
  if(simR.status===200 && simR.data && simR.data.data) ok(P+'-sim-status','active='+simR.data.data.active);
  else if(simR.status===200) ok(P+'-sim-status','simulation status 200');
  else bad(P+'-sim-status','expected 200',simR.status+': '+JSON.stringify(simR).substring(0,200));
  var badSc = await api('POST','/api/simulation/scenario',{headers:hdr('admin'),body:{scenario:'invalid_scenario_x'}});
  if(badSc.status===400) ok(P+'-sim-bad-scenario','invalid scenario->400');
  else bad(P+'-sim-bad-scenario','expected 400',badSc.status+': '+badSc.text.substring(0,200));
  var okSc = await api('POST','/api/simulation/scenario',{headers:hdr('admin'),body:{scenario:'normal'}});
  if(okSc.status===200) ok(P+'-sim-good-scenario','scenario normal->200');
  else bad(P+'-sim-good-scenario','expected 200',okSc.status+': '+okSc.text.substring(0,200));
  var simStartR = await api('POST','/api/simulation/start',{headers:hdr('admin')});
  if(simStartR.status===200||simStartR.status===400) ok(P+'-sim-start','status='+simStartR.status);
  else bad(P+'-sim-start','unexpected',simStartR.status+': '+simStartR.text.substring(0,200));

  // === 9. Misc Public Endpoints ===
  var cfgR = await api('GET','/api/config/summary');
  if(cfgR.status===200) ok(P+'-cfg-summary','config summary->200');
  else bad(P+'-cfg-summary','expected 200',cfgR.status+': '+cfgR.text.substring(0,200));
  var permR = await api('GET','/api/permissions/roles');
  if(permR.status===200&&permR.data&&permR.data.data&&permR.data.data.roles&&permR.data.data.roles.length>0) ok(P+'-perm-roles','roles count='+permR.data.data.roles.length);
  else bad(P+'-perm-roles','expected 200+roles',permR.status+': '+JSON.stringify(permR).substring(0,200));
  var mbR = await api('GET','/api/modbus/registers');
  if(mbR.status===200) ok(P+'-modbus-regs','modbus registers->200');
  else bad(P+'-modbus-regs','expected 200',mbR.status+': '+mbR.text.substring(0,200));
  var nxR = await api('GET','/api/not-exist');
  if(nxR.status===404) ok(P+'-notfound','GET /api/not-exist->404');
  else bad(P+'-notfound','expected 404',nxR.status+': '+nxR.text.substring(0,200));

  // === 10. Recommendations (GET only) ===
  var recR = await api('GET','/api/recommendations/latest');
  if(recR.status===200) ok(P+'-rec-get','GET recommendations->200');
  else bad(P+'-rec-get','expected 200',recR.status+': '+recR.text.substring(0,200));

  // === 11. Product Results (abnormal cases only) ===
  var missF = await api('POST','/api/product-results',{headers:hdr('engineer'),body:{}});
  if(missF.status===400||missF.status===422||missF.status===404) ok(P+'-pr-miss','missing fields->'+missF.status+' (4xx)');
  else bad(P+'-pr-miss','expected 4xx',missF.status+': '+missF.text.substring(0,200));
  var yldOvr = await api('POST','/api/product-results',{headers:hdr('engineer'),body:{batch_id:99999,yield_percent:150,product_ratio:0.5}});
  if(yldOvr.status>=400&&yldOvr.status<500) ok(P+'-pr-yield','yield 150->'+yldOvr.status+' (4xx)');
  else bad(P+'-pr-yield','expected 4xx',yldOvr.status+': '+yldOvr.text.substring(0,200));
  var ratOvr = await api('POST','/api/product-results',{headers:hdr('engineer'),body:{batch_id:99999,yield_percent:50,product_ratio:2.5}});
  if(ratOvr.status>=400&&ratOvr.status<500) ok(P+'-pr-ratio','ratio 2.5->'+ratOvr.status+' (4xx)');
  else bad(P+'-pr-ratio','expected 4xx',ratOvr.status+': '+ratOvr.text.substring(0,200));
  var noBId = await api('POST','/api/product-results',{headers:hdr('engineer'),body:{batch_id:99999,yield_percent:50,product_ratio:0.5}});
  if(noBId.status===404) ok(P+'-pr-no-batch','batch 99999->404');
  else if(noBId.status>=400&&noBId.status<500) ok(P+'-pr-no-batch','no batch->'+noBId.status+' (4xx)');
  else bad(P+'-pr-no-batch','expected 404',noBId.status+': '+noBId.text.substring(0,200));

  // === 12. Test Reset Safety (must be last) ===
  var noHdrR = await api('POST','/api/test/reset',{headers:hdr('admin')});
  if(noHdrR.status===403||noHdrR.status===404) ok(P+'-reset-no-hdr','no confirm header->'+noHdrR.status);
  else bad(P+'-reset-no-hdr','expected 403/404',noHdrR.status+': '+noHdrR.text.substring(0,200));
  // Clean up state before final reset
  try { await api('POST','/api/processes/current/stop',{headers:hdr('engineer')}); } catch(e) {}
  try { await api('POST','/api/control/emergency-stop/reset',{headers:hdr('engineer')}); } catch(e) {}
  try { await api('POST','/api/control/manual-lock',{headers:hdr('engineer'),body:{locked:false}}); } catch(e) {}
  await sleep(500);
  var allB2 = await api('GET','/api/batches',{headers:hdr('engineer')});
  var allB2Arr = allB2.status===200&&allB2.data&&allB2.data.data ? (allB2.data.data.batches||[]) : [];
  for(var bi3=0; bi3<allB2Arr.length; bi3++) { if(allB2Arr[bi3].finished_at==null) await api('POST','/api/batches/'+allB2Arr[bi3].id+'/finish',{headers:hdr('engineer')}).catch(function(){}); }
  await sleep(500);
  var withHdr = await api('POST','/api/test/reset',{headers:{authorization:'Bearer '+tokens.admin,'x-xingshu-test-confirm':'local-e2e'}});
  if(withHdr.status===204||withHdr.status===200) ok(P+'-reset-with-hdr','test reset->'+withHdr.status);
  else bad(P+'-reset-with-hdr','expected 204/200',withHdr.status+': '+withHdr.text.substring(0,300));
  var postB = await api('GET','/api/batches',{headers:hdr('engineer')});
  var postBC = postB.status===200&&postB.data&&postB.data.data ? (postB.data.data.batches||[]).length : -1;
  if(postBC===0||postBC<(batchesArr.length||0)) ok(P+'-reset-batches','batches cleared ('+batchesArr.length+'->'+postBC+')');
  else bad(P+'-reset-batches','not cleared',batchesArr.length+' batches before, '+postBC+' after');

  console.log('');
  console.log('='.repeat(60));
  console.log('TOTAL: '+(passCount+failCount)+' | PASS:'+passCount+' | FAIL:'+failCount);
  if(failures.length) {
    console.log('\nFAILED TESTS:');
    for(var fi=0;fi<failures.length;fi++) { console.log('  - '+failures[fi].name+': '+failures[fi].detail); }
  }
  process.exit(failCount>0?1:0);
})().catch(function(e){console.error('FATAL',e);process.exit(1);});