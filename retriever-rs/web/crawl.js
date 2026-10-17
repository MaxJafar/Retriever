/* Shared coordinator schedules; this view never owns a timer that starts jobs. */
(() => {
  const node = (tag, text, cls) => { const e = document.createElement(tag); if (text) e.textContent = text; if (cls) e.className = cls; return e; };
  const button = (text, action) => { const b = node('button', text); b.type = 'button'; b.addEventListener('click', action); return b; };
  const dialog = node('dialog', null, 'workspaceDialog crawlDialog');
  dialog.id = 'crawlDialog'; dialog.setAttribute('aria-labelledby', 'crawlTitle');
  const heading = node('div', null, 'wizardHeading'); const title = node('h2', 'Crawling & learning'); title.id = 'crawlTitle';
  heading.append(title, button('Close', () => dialog.close()));
  const intro = node('p', 'Retriever builds a local catalogue, then checks for changes on your schedule. Unchanged files reuse their extracted text, descriptions, and embeddings.');
  const disclosure = node('p', null, 'hint');
  const progress = node('p'); progress.setAttribute('role', 'status'); progress.setAttribute('aria-live', 'polite');
  const folders = node('div', null, 'crawlFolders');
  const controls = node('div', null, 'folderActions');
  const refresh = button('Refresh status', () => load().catch(showError));
  const cancel = button('Cancel current job', () => act(() => api('/api/jobs/cancel', 'POST', {})));
  controls.append(refresh, cancel);
  const error = node('p', null, 'error'); error.setAttribute('role', 'alert');
  dialog.append(heading, intro, disclosure, controls, progress, folders, error); document.body.append(dialog);
  const cards = new Map();
  const time = value => value > 0 ? new Date(value * 1000).toLocaleString() : 'Not yet';
  const key = f => JSON.stringify([f.device, f.root]);
  const showError = e => { error.textContent = e.message || String(e); };
  async function act(action) { try { error.textContent = ''; await action(); await load(); } catch (e) { showError(e); } }
  function card(f) {
    const el = node('section', null, 'crawlFolder');
    el.append(node('h3', f.root), node('p', f.label, 'hint'));
    const settings = node('div', null, 'crawlChoices');
    const scheduleLabel = node('label', 'Check for changes'); const schedule = node('select');
    for (const [value, label] of [[0,'Manual'],[15,'Every 15 minutes'],[60,'Hourly'],[360,'Every 6 hours'],[1440,'Daily'],[10080,'Weekly'],[-1,'Custom…']]) { const option = node('option',label); option.value=value; schedule.append(option); }
    const customLabel = node('label', 'Minutes'); const custom = node('input'); custom.type='number'; custom.min='15'; custom.max='43200'; custom.step='1'; customLabel.append(custom);
    const analysisLabel = node('label','Learn from files'); const analysis=node('select');
    for (const [value,label] of [['off','Native retrieval · EmbeddingGemma + Laya']]) {const option=node('option',label);option.value=value;analysis.append(option);}
    scheduleLabel.append(schedule); analysisLabel.append(analysis);settings.append(scheduleLabel,customLabel,analysisLabel);el.append(settings);
    const state=node('p',null,'hint');state.setAttribute('aria-live','polite');
    const actions=node('div',null,'folderActions');
    const c={el,state,schedule,analysis,custom,customLabel,dirty:false,saved:f};
    const dirty=()=>{c.dirty=true;customLabel.hidden=schedule.value!=='-1';save.disabled=false;};
    schedule.addEventListener('change',dirty);analysis.addEventListener('change',dirty);custom.addEventListener('input',dirty);
    const save=button('Save folder settings',()=>act(async()=>{
      const minutes=Number(schedule.value==='-1'?custom.value:schedule.value);
      if (!Number.isInteger(minutes) || (minutes!==0 && (minutes<15 || minutes>43200))) throw new Error('Choose manual or 15–43200 whole minutes.');
      await api('/api/crawl/settings','POST',{device:f.device,root:f.root,revision:c.saved.revision,interval_minutes:minutes,analysis:analysis.value});c.dirty=false;
    }));save.disabled=true;c.save=save;
    const run=button('Crawl now',()=>act(()=>api('/api/crawl/run','POST',{device:f.device,root:f.root})));c.run=run;
    actions.append(save,run);el.append(state,actions);return c;
  }
  async function load() {
    const result=await api('/api/crawl');disclosure.textContent=result.disclosure;
    cancel.disabled=!result.running;
    progress.textContent=result.running?`Working · ${result.progress?.phase||'starting'}. Search stays available.`:'Schedules run while the coordinator is open. Overdue runs are combined into one check per folder.';
    const active=new Set();
    for (const f of result.folders) {
      const id=key(f);active.add(id);let c=cards.get(id);
      if(!c){c=card(f);cards.set(id,c);folders.append(c.el);}
      if(!c.dirty){c.saved=f;c.schedule.value=[0,15,60,360,1440,10080].includes(f.interval_minutes)?String(f.interval_minutes):'-1';c.custom.value=f.interval_minutes||60;c.customLabel.hidden=c.schedule.value!=='-1';c.analysis.value=f.analysis;c.save.disabled=true;}
      const r=f.report||{};const failed=r.errors?.length?` · ${r.errors.length} issue(s): ${r.errors[0].reason||'Check source access'}`:'';
      c.state.textContent=`${f.state} · Last completed: ${time(f.last_completed)} · Next: ${f.interval_minutes===0?'Manual':f.next_run>0?time(f.next_run):'Due'} · ${r.changed||0} changed, ${r.unchanged||0} reused${failed}`;
      c.run.disabled=result.running;
    }
    for(const [id,c] of cards) if(!active.has(id)){c.el.remove();cards.delete(id);}
    if(!result.folders.length)progress.textContent='Include a folder in Settings to choose its crawl schedule.';
  }
  const open=async()=>{document.getElementById('indexDialog')?.close();dialog.showModal();try{await load();}catch(e){showError(e);}};
  document.querySelector('footer')?.insertBefore(button('Crawling & learning',open),document.getElementById('keyboardHelp'));
  document.getElementById('indexDialog')?.append(button('Folder schedules & learning',open));
  window.setInterval(()=>{if(dialog.open&&!document.hidden)load().catch(showError);},5000);
})();
