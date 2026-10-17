'use strict';
(() => {
  const cache = new Map(), waiting = new Map(); let scheduled = false;
  const labels = {'.txt':'Text document','.md':'Markdown document','.pdf':'PDF document','.doc':'Word document','.docx':'Word document','.xls':'Excel workbook','.xlsx':'Excel workbook','.csv':'CSV file','.pptx':'PowerPoint presentation','.zip':'ZIP archive','.json':'JSON file','.js':'JavaScript file','.ts':'TypeScript file','.py':'Python file','.png':'PNG image','.jpg':'JPEG image','.jpeg':'JPEG image','.mp3':'MP3 audio','.mp4':'MP4 video'};
  const keyFor = row => {
    if(row.kind==='folder')return 'folder';
    const value=row.extension||row.name?.match(/\.[^.]+$/)?.[0]||'',extension=value.startsWith('.')?value:'.'+value;
    return /^\.[a-z0-9_+-]{1,24}$/i.test(extension)?extension.toLowerCase():'file';
  };
  function fallback(key) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg','svg');
    svg.setAttribute('viewBox','0 0 32 32'); svg.setAttribute('aria-hidden','true');
    const shape = (tag, attributes) => {const node=document.createElementNS(svg.namespaceURI,tag);for(const [k,v] of Object.entries(attributes))node.setAttribute(k,v);svg.append(node);};
    if(key==='folder') {
      shape('path',{d:'M2 8a2 2 0 0 1 2-2h9l3 3h12a2 2 0 0 1 2 2v15H2Z',fill:'#be870f'});
      shape('path',{d:'M2 12h28l-2 14H4Z',fill:'#edbf48',stroke:'#e1ab2f','stroke-width':'.6'});
      shape('path',{d:'M3 12h26',stroke:'#ffdc76','stroke-width':'1.5'});
    } else {
      const color=/^\.(png|jpe?g|gif|webp|svg)$/.test(key)?'#5a9f91':/^\.(xlsx?|csv)$/.test(key)?'#569369':/^\.(pdf|pptx?)$/.test(key)?'#ce7669':/^\.(mp[34]|wav|mov)$/.test(key)?'#a081b6':'#7393b6';
      shape('path',{d:'M7 2h12l7 7v21H7Z',fill:'#eee',stroke:'#b2b2b2','stroke-width':'.6'});
      shape('path',{d:'M19 2v7h7',fill:'#c8cbd1'});
      shape('rect',{x:'9',y:'14',width:'13',height:'11',rx:'1',fill:color});
      shape('path',{d:'M12 17h7m-7 3h7m-7 3h4',stroke:'#fff','stroke-width':'1.2',fill:'none'});
    }
    return svg;
  }
  function apply(node, data) {
    if (!data || !/^data:image\/png;base64,[A-Za-z0-9+/=]+$/.test(data.src || '')) return;
    const image = document.createElement('img');image.src=data.src;image.alt='';image.draggable=false;
    image.addEventListener('error',()=>node.replaceChildren(fallback(node.dataset.iconKey)),{once:true});
    node.replaceChildren(image);node.dataset.native='true';
  }
  async function flush() {
    scheduled=false;const batch=[...waiting.keys()].slice(0,64);if(!batch.length)return;
    const nodes=new Map(batch.map(key=>[key,waiting.get(key)]));batch.forEach(key=>waiting.delete(key));
    let result;
    try {
      const nativeAvailable=!!(bridge||window.webkit?.messageHandlers?.searchx||window.chrome?.webview);
      if(nativeAvailable)try{result=await native('file-icons',{extensions:batch});}catch{}
      if(!result)result=await api('/api/file-icons','POST',{extensions:batch});
    } catch {result={icons:{}};}
    for(const key of batch){const data=result?.icons?.[key]||{};cache.set(key,data);for(const node of nodes.get(key))apply(node,data);}
    if(cache.size>256)cache.delete(cache.keys().next().value);
    window.dispatchEvent(new Event('retrieverIconsReady'));
    if(waiting.size&&!scheduled){scheduled=true;queueMicrotask(flush);}
  }
  function create(row, cls='') {
    const key=keyFor(row),node=document.createElement('span');node.className='systemFileIcon '+cls;node.dataset.iconKey=key;node.setAttribute('aria-hidden','true');node.append(fallback(key));
    if(cache.has(key))apply(node,cache.get(key));
    else {if(!waiting.has(key))waiting.set(key,[]);waiting.get(key).push(node);if(!scheduled){scheduled=true;queueMicrotask(flush);}}
    return node;
  }
  function label(row) {const key=keyFor(row);return key==='folder'?'File folder':labels[key]||cache.get(key)?.label||(key==='file'?'File':key.slice(1).toUpperCase()+' file');}
  window.retrieverFileIcons={create,label};
})();

'use strict';
const $ = id => document.getElementById(id);
let bridge = null, serial = 0, pending = new Map(), cfg = null, configRevision = '', step = 0;
let kind = '', hits = [], selected = 0, sequence = 0, timer, current = null, pollTimer;
let layaState = '', latestStatus = null, savedSnapshot = '', planRequest = 0, searchPending = false, renderedQuery = '';
let folderPicker = null, folderRequest = 0, folderFilterTimer;
const extractionDetails = {pdf_password_required:'PDF needs its opening password; filename indexed.',pdf_encryption_unsupported:'PDF encryption is unsupported; filename indexed.',pdf_invalid:'PDF could not be opened; filename indexed.',pdf_crypto_dependency_missing:'Install the PDF crypto extra to read this encrypted PDF; filename indexed.',pdf_text_extraction_failed:'PDF text extraction failed; filename indexed.'};
function contentDetail(detail) { return extractionDetails[(detail || '').replace(/^metadata: /,'')] || ''; }
function element(tag, value = '', cls = '') { const e = document.createElement(tag); e.textContent = value; e.className = cls; return e; }
const modelRoles = ['embedding','relevance'];
function gatherModel(purpose) { return {...cfg.search[purpose], provider:'native'}; }
function fillModelForms() { if($('nativeStorage')) $('nativeStorage').textContent = latestStatus?.storage?.root || 'Application data folder'; }
window.searchxReceive = raw => { const message = typeof raw === 'string' ? JSON.parse(raw) : raw; const p = pending.get(message.id); if (p) { pending.delete(message.id); clearTimeout(p.timeout); message.error ? p.reject(new Error(message.error)) : p.resolve(message.data); } };
function native(action, args = {}) {
  if (!window.webkit?.messageHandlers?.searchx && !bridge && !window.chrome?.webview) return Promise.reject(new Error('This action requires the desktop app'));
  const id = ++serial;
  return new Promise((resolve, reject) => { const timeout = setTimeout(() => { pending.delete(id); reject(new Error('Request timed out')); }, 150000); pending.set(id, {resolve, reject, timeout}); const m = {...args, id, action}; if (bridge) bridge.request(JSON.stringify(m)); else if(window.chrome?.webview)window.chrome.webview.postMessage(m);else window.webkit.messageHandlers.searchx.postMessage(m); });
}
if(window.chrome?.webview)window.chrome.webview.addEventListener('message',event=>window.searchxReceive(event.data));
async function api(path, method = 'GET', body, revision) {
  if (bridge || window.webkit?.messageHandlers?.searchx) { try { return await native('api', {path, method, body, revision}); } catch (e) { if (/sign in|pairing token|not signed/i.test(e.message)) showLogin(); throw e; } }
  const options = {method, credentials: 'same-origin', headers: {}};
  if (body !== undefined) { options.headers['Content-Type'] = 'application/json'; options.body = JSON.stringify(body); }
  if (revision) options.headers['If-Match'] = revision;
  let response;
  try { response = await fetch(path, options); } catch (e) { throw new Error('Cannot reach Retriever. Check that the main PC is running, then try again.'); }
  let data;
  try { data = await response.json(); } catch (e) { throw new Error('Retriever returned an unexpected response. Reopen the app or check the server terminal.'); }
  if (!response.ok) { if (response.status === 401) showLogin(); const error = new Error(data.error || 'Request failed'); error.status = response.status; throw error; }
  return data;
}
// One active request and one replaceable pending request per lane. Aborting
// fetch cannot cancel local inference, so never dispatch a backlog of old input.
const searchLanes = new Map();
function cancelQueuedSearch(owner) {
  for (const lane of searchLanes.values()) if (lane.pending?.owner === owner) {
    lane.pending.reject(new DOMException('Search superseded', 'AbortError')); lane.pending = null;
  }
}
function requestSearch(params, phase, owner) {
  const lane = searchLanes.get(phase) || {active:null,pending:null}; searchLanes.set(phase,lane);
  return new Promise((resolve,reject) => {
    const key = JSON.stringify(params), entry = {params,phase,owner,resolve,reject,key,followers:[]};
    if(lane.active?.key===key) {lane.active.followers.push({resolve,reject});return;}
    if (lane.active) {
      lane.pending?.reject(new DOMException('Search superseded', 'AbortError')); lane.pending = entry;
    } else dispatchSearch(lane,entry);
  });
}
async function dispatchSearch(lane,entry) {
  lane.active = entry;
  try { const data=await api('/api/search?' + new URLSearchParams({...entry.params,phase:entry.phase}));entry.resolve(data);for(const follower of entry.followers)follower.resolve(data); }
  catch(error) { entry.reject(error);for(const follower of entry.followers)follower.reject(error); }
  finally { lane.active = null; const next = lane.pending; lane.pending = null; if(next) dispatchSearch(lane,next); }
}
function searchBusy(value) { $('searchActivity').hidden = !value; $('commandSearch').classList.toggle('is-searching',value); }
function cancelSearch() { ++sequence; clearTimeout(timer); cancelQueuedSearch('main'); searchPending=false; searchBusy(false); }
window.retrieverSearch = {cancel:cancelSearch};
function showLogin() { cancelSearch(); if(document.body.classList.contains('quick-mode')){window.retrieverSetQuickMode?.(false);native('workspace').catch(()=>{});}window.searchxWorkspace?.hide(); document.querySelectorAll('dialog[open]').forEach(d=>d.close()); if ($('folderBrowser').open) closeFolderBrowser(); if ($('indexDialog').open) $('indexDialog').close(); $('showIndex').hidden = true; $('login').hidden = false; $('searchPane').hidden = true; $('wizard').hidden = true; }
async function signIn(token) { await api('/api/login', 'POST', {token}); $('token').value = ''; $('login').hidden = true; $('searchPane').hidden = false; await status(true); }
$('signIn').onclick = async () => { $('loginError').textContent = ''; if (!$('token').value.trim()) { $('loginError').textContent = 'Enter your main PC’s pairing token.'; return; } $('signIn').disabled = true; try { await signIn($('token').value.trim()); } catch (e) { $('loginError').textContent = e.message; } finally { $('signIn').disabled = false; } };
$('token').onkeydown = e => { if (e.key === 'Enter') $('signIn').click(); };
async function status(initial = false) {
  try {
    const s = await api('/api/status'); latestStatus = s; $('showIndex').hidden = s.role === 'client';
    window.searchxWorkspace?.status(s,initial);
    for (const [id,values] of [['category',s.analysis?.categories],['topic',s.analysis?.tags]]) {
      const control = $(id), previousValue = control.value;
      control.replaceChildren(new Option(id === 'category' ? 'All AI categories' : 'All AI topics',''));
      Object.entries(values || {}).forEach(([name,count]) => control.append(new Option(`${name} (${count})`,name)));
      control.value = previousValue; control.parentElement.hidden = !s.analysis?.files;
    }
    const layaFinished = layaState === 'loading' && s.laya?.state !== 'loading';
    layaState = s.laya?.state || '';
    const previous = $('device').value;
    $('device').replaceChildren(new Option('All devices', ''));
    for (const d of s.devices || []) if (d.enabled) $('device').append(new Option(d.label + (d.status === 'offline' ? ' · offline' : ''), d.id));
    $('device').value = previous;
    const indexing = s.last_sync?.phase === 'analyzing' ? 'Analyzing opted-in files on the main PC…' : s.last_sync?.phase === 'embeddings' ? 'Building the AI index on this PC…' : 'Scanning selected folders…';
    const contentWarnings = (s.devices || []).reduce((total,d) => total + (d.enabled ? Object.values(d.report?.warning_counts || {}).reduce((n,v) => n + v,0) : 0),0);
    const deviceCount = (s.devices || []).filter(d => d.enabled).length;
    $('health').textContent = s.syncing ? indexing : `${s.files.toLocaleString()} files · ${deviceCount} device${deviceCount === 1 ? '' : 's'}`;
    const issues = (s.devices || []).filter(d => d.enabled && ['offline', 'partial'].includes(d.status));
    if (!s.syncing && issues.length) $('health').textContent += ` · ${issues.length} offline / partial`;
    if (!s.syncing && contentWarnings) $('health').textContent += ` · ${contentWarnings} content warnings`;
    const warningNote = contentWarnings ? ` ${contentWarnings} PDFs have filename search only; their results explain the extraction issue.` : '';
    $('indexState').textContent = s.syncing ? indexing + ' You can continue searching.' : `${s.files.toLocaleString()} files indexed. ${issues.length ? issues.map(d => `${d.label}: ${d.status}`).join(' · ') : s.files ? 'All selected folders were scanned.' : 'Select folders and start your first scan.'}${warningNote}`;
    $('sync').disabled = s.syncing || !s.can_index;
    $('sync').textContent = s.syncing ? 'Indexing…' : 'Index selected folders';
    $('statusdot').dataset.state = s.syncing ? 'working' : issues.length ? 'warning' : 'ready';
    renderProgress(s);
    renderProgress(s,'indexOverview'); $('scanSummary').textContent = $('indexState').textContent;
    $('scanNow').disabled = s.syncing || !s.can_index; $('scanNow').textContent = s.syncing ? 'Indexing…' : 'Index selected folders';
    $('rebuildMeaning').disabled = s.syncing || !s.can_index || !s.files;
    $('rebuildMeaning').hidden = s.role === 'client';
    if (initial && s.configured && location.hash !== '#setup') {
      const welcome = $('results').querySelector('.welcome');
      if (welcome) { welcome.querySelector('h1').textContent = 'Your files. One search.'; welcome.querySelector('p').textContent = `${s.files.toLocaleString()} files across ${(s.devices || []).filter(d => d.enabled).length} device${(s.devices || []).filter(d => d.enabled).length === 1 ? '' : 's'}. Search a filename, phrase or idea above.`; $('welcomeSetup').textContent = 'Manage devices and folders'; }
    }
    if (initial && (!s.configured || location.hash === '#setup')) await openSetup();
    if(initial&&s.configured){window.retrieverReady=true;window.dispatchEvent(new Event('retrieverReady'));}
    clearTimeout(pollTimer); pollTimer = setTimeout(status, s.syncing || layaState === 'loading' ? 2000 : 15000);
    if (layaFinished && $('query').value.trim() && $('wizard').hidden && $('preview').hidden && !$('searchPane').hidden) search();
  } catch (e) { $('health').textContent = e.message; $('statusdot').dataset.state = 'warning'; clearTimeout(pollTimer); pollTimer = setTimeout(status,5000); }
}
function renderProgress(s, target = 'deviceProgress') {
  $(target).replaceChildren();
  for (const error of s.last_sync?.errors || []) $(target).append(element('p',`${error.id ? (s.devices.find(d => d.id === error.id)?.label || error.id) : 'Meaning index'}: ${error.error}`,'error path'));
  for (const d of s.devices || []) { if (!d.enabled || s.role === 'client') continue; const card = element('div','','progressCard'); const active = s.syncing && s.last_sync?.current_device === d.id && s.last_sync?.phase === 'collecting';
    card.append(element('strong',d.label),element('span',active ? 'Scanning…' : ({ready:'Ready',offline:'Offline · previous results retained',partial:'Partial scan · check access',pending:'Not scanned yet'})[d.status] || 'Not scanned yet','badge'),element('p',`${d.files.toLocaleString()} indexed files`));
    if (active && s.last_sync?.progress) card.append(element('p',`${s.last_sync.progress.collected.toLocaleString()} collected so far`,'hint'));
    const errors = d.report?.errors || []; if (errors.length) { const details = element('details'); details.append(element('summary','Scan details')); for (const error of errors.slice(0,5)) details.append(element('p',error.path ? `${error.path}: ${error.reason}` : error.reason || String(error),'path')); card.append(details); } $(target).append(card);
  }
}
function render(data) {
  const previousId = hits[selected]?.id;
  searchPending = false; renderedQuery = $('query').value.trim(); $('results').setAttribute('aria-busy','false');
  hits = data.results || []; selected = Math.max(0,hits.findIndex(hit=>hit.id===previousId)); $('results').replaceChildren();
  $('count').textContent = `${hits.length} result${hits.length === 1 ? '' : 's'}` + (data.elapsed_ms ? ` · ${Math.round(data.elapsed_ms)} ms` : '');
  const rerank = data.reranking || {};
  if (rerank.state === 'loading') { layaState = 'loading'; clearTimeout(pollTimer); pollTimer = setTimeout(status,1000); }
  const rankLabel = rerank.provider === 'laya' ? 'Laya' : 'Your relevance model';
  const rankNote = rerank.applied ? rankLabel + ' checked the relevance of shortlisted passages.' : ({loading:'Laya is warming up; showing the current search order.',missing_runtime:'Install the optional Laya runtime on the coordinator to use relevance ordering.',missing_model:'Choose a complete local Laya checkpoint in settings.',missing_english_model:'The optional English Laya checkpoint is incomplete.',busy:rankLabel + ' is busy; showing the current search order.',api_failed:'The relevance API could not finish or returned invalid decisions. Showing retrieval order; test the model connection in settings.',timeout_or_inference_failed:'Laya could not finish in time; showing the current search order.',model_load_failed:'Laya could not load the checkpoint; run laya-check on the coordinator.',worker_failed:'Laya could not start; run laya-check on the coordinator.'})[rerank.state];
  $('searchNote').textContent = [data.warning || data.note, rankNote].filter(Boolean).join(' '); $('searchNote').hidden = !$('searchNote').textContent;
  if (!hits.length) { $('results').append(element('div', data.warning ? 'No results are available for this query.' : 'No matches. Try another filename or phrase.', 'empty')); return; }
  for (const [i, h] of hits.entries()) {
    const row = element('div', '', 'hit' + (i === selected ? ' selected' : '')); row.tabIndex = 0; row.setAttribute('role', 'button'); row.onclick = () => { selected = i; preview(h); }; row.onkeydown = e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selected = i; preview(h); } };
    row.append(window.retrieverFileIcons.create(h,'fileicon'));
    const body = element('div', '', 'hitbody'); body.append(element('h3', h.name), element('p', h.snippet || contentDetail(h.detail) || 'Filename indexed; content extraction is optional for this format.'));
    const metadata = element('div', '', 'metadata'); metadata.append(element('span', latestStatus?.devices.find(d => d.id === h.device)?.label || h.device, 'badge'), element('span', h.type));
    if (h.source_status && h.source_status !== 'ready') metadata.append(element('span', `Last indexed copy · ${h.source_status}`, 'weak'));
    if (h.weak) metadata.append(element('span', 'Partial match', 'weak'));
    if (contentDetail(h.detail)) metadata.append(element('span', 'Content unavailable', 'weak'));
    if (h.laya) metadata.append(element('span', 'Laya checked', 'badge'));
    if (h.analysis) metadata.append(element('span',`AI category · ${h.analysis.category}`,'badge'));
    if (h.generated) metadata.append(element('span','AI description','badge'));
    if (h.relevance) metadata.append(element('span', 'Relevance checked', 'badge'));
    body.append(metadata); row.append(body); $('results').append(row);
  }
}
async function search() {
  if (!$('wizard').hidden || !$('login').hidden) return;
  clearTimeout(timer); cancelQueuedSearch('main');
  const n = ++sequence, query = $('query').value.trim();
  window.searchxWorkspace?.search(); back(); hits = []; renderedQuery = '';
  $('clearSearch').hidden = !query; $('searchNote').hidden = true;
  if (!query) {
    searchPending=false; searchBusy(false); $('results').setAttribute('aria-busy','false');
    $('results').replaceChildren(element('div','Search by filename, a phrase, or the idea inside a file.','empty')); $('count').textContent=''; return;
  }
  const params = {q:query,kind,device:$('device').value,mode:$('mode').value,category:$('category').value,tag:$('topic').value};
  const progressive = latestStatus?.search_routing?.version >= 1 && params.mode !== 'semantic';
  searchPending=true; searchBusy(true); $('results').setAttribute('aria-busy','true');
  $('count').textContent='Searching…'; $('results').replaceChildren(element('div','Finding your files…','empty'));
  async function run(phase) {
    try {
      const data = await requestSearch(params,phase,'main');
      if(n !== sequence) return;
      render(data); searchBusy(false);
      if(phase==='instant' && params.mode!=='exact' && data.effective_mode!=='exact') {
        $('count').append(document.createTextNode(' · Checking meaning…'));
        searchBusy(true); timer=setTimeout(()=>{if(n===sequence)run('full');},420);
      }
    } catch(error) {
      if(n!==sequence || error.name==='AbortError') return;
      searchPending=false; searchBusy(false); $('results').setAttribute('aria-busy','false');
      if(!hits.length) $('results').replaceChildren(element('div',error.message,'empty'));
      $('count').textContent=error.message;
    }
  }
  await run(progressive?'instant':'full');
}
async function preview(h) {
  current = h; $('results').hidden = true; $('preview').hidden = false; $('previewTitle').textContent = h.name; $('previewPath').textContent = (latestStatus?.devices.find(d => d.id === h.device)?.label || h.device) + ' · ' + h.path; $('previewText').textContent = h.snippet || '';
  $('open').hidden = !(bridge || window.webkit?.messageHandlers?.searchx || window.chrome?.webview); $('open').disabled = h.device !== (window.searchxDevice || latestStatus?.local_device); $('copy').textContent = 'Copy path';
  try { const result = await api('/api/result/' + encodeURIComponent(h.id)); if (current === h) $('previewText').textContent = result.text || contentDetail(result.detail) || h.snippet || result.detail; } catch (e) { if (current === h) $('previewText').textContent = e.message; }
}
function back() { current = null; $('preview').hidden = true; $('results').hidden = false; }
$('back').onclick = back;
$('query').oninput = event => {
  cancelSearch(); hits=[]; renderedQuery=''; $('clearSearch').hidden=!$('query').value;
  back(); $('searchNote').hidden=true; $('count').textContent='';
  const hint=$('task').value==='search'?($('query').value.trim()?'Finding your files…':'Search by filename, a phrase, or the idea inside a file.'):$('task').value==='chat'?'Press Enter to send your message.':'Press Enter to ask about your indexed files.';
  $('results').replaceChildren(element('div',hint,'empty'));
  if(event?.isComposing) return;
  if($('wizard').hidden && $('login').hidden) window.searchxWorkspace?.search();
  if($('task').value==='search') timer=setTimeout(search,180);
};
$('query').addEventListener('compositionend',()=>$('query').oninput());
$('commandSearch').onsubmit = event => {event.preventDefault();clearTimeout(timer);
  $('task').value==='search' && hits.length && !searchPending && renderedQuery===$('query').value.trim() ? preview(hits[selected]) : search();
};
$('clearSearch').onclick = () => {$('query').value='';$('query').oninput();$('query').focus();};
$('query').onkeydown = event => {
  if(event.key==='Escape') {event.preventDefault();event.stopPropagation();
    if($('query').value) $('clearSearch').click(); else {cancelSearch();window.searchxWorkspace?.show();$('query').blur();}
  }
};
function updateSearchFilter() { cancelSearch(); search(); }
$('device').onchange = updateSearchFilter; $('mode').onchange = updateSearchFilter;
$('category').onchange = updateSearchFilter; $('topic').onchange = updateSearchFilter;
document.querySelectorAll('[data-kind]').forEach(button => button.onclick = () => { kind = button.dataset.kind; document.querySelector('[data-kind].active')?.classList.remove('active'); button.classList.add('active'); updateSearchFilter(); });
$('copy').onclick = () => { if (current) (bridge || window.webkit?.messageHandlers?.searchx || window.chrome?.webview ? native('copy',{text:current.path}) : navigator.clipboard.writeText(current.path)).then(() => $('copy').textContent = 'Copied').catch(e => $('previewText').textContent = e.message); };
$('open').onclick = () => { if (current) native('open',{hit:current}).catch(e => $('previewText').textContent = e.message); };
function field(label, value, change, type = 'text') { const wrapper = element('label',label); const input = document.createElement('input'); input.type = type; input.value = value; input.oninput = () => change(input.value); wrapper.append(input); return {wrapper,input}; }
async function openSetup() {
  cancelSearch();
  window.searchxWorkspace?.hide();
  try {
    const data = await api('/api/config'); cfg = data.config; configRevision = data.revision;
    $('login').hidden = true; $('searchPane').hidden = true; $('wizard').hidden = false; $('setupForm').hidden = false; $('setupDone').hidden = true;
    document.querySelectorAll('[name=role]').forEach(e => e.checked = e.value === cfg.role);
    $('endpoint').value = cfg.endpoint; $('sshConfig').value = cfg.ssh_config;
    $('maxFile').value = cfg.limits.max_file_bytes / 1048576; $('maxMedia').value = (cfg.limits.max_media_bytes ?? cfg.limits.max_file_bytes) / 1048576; $('maxTotal').value = cfg.limits.max_total_bytes / 1048576;
    $('maxFiles').value = cfg.limits.max_files; $('resultLimit').value = cfg.search.limit;
    $('maxText').value = cfg.limits.max_text_chars / 1048576;
    $('alwaysOnTop').checked = cfg.appearance.always_on_top;
    fillModelForms();
    $('setupTitle').textContent = cfg.configured ? 'Retriever settings' : 'Set up Retriever'; $('reloadSettings').hidden = true; $('steps').hidden = false;
    renderDevices(); renderFolderChoices(); savedSnapshot = JSON.stringify(gather()); $('draftState').textContent = 'Changes are saved at the final step.'; showStep(0);
  } catch (e) { $('health').textContent = e.message; }
}
function gather() {
  cfg.role = document.querySelector('[name=role]:checked').value;
  cfg.endpoint = $('endpoint').value.trim(); cfg.ssh_config = $('sshConfig').value.trim();
  cfg.limits.max_file_bytes = Number($('maxFile').value) * 1048576; cfg.limits.max_media_bytes = Number($('maxMedia').value) * 1048576; cfg.limits.max_total_bytes = Number($('maxTotal').value) * 1048576;
  cfg.limits.max_files = Number($('maxFiles').value); cfg.search.limit = Number($('resultLimit').value);
  cfg.limits.max_text_chars = Number($('maxText').value) * 1048576;
  cfg.appearance.always_on_top = $('alwaysOnTop').checked;
  delete cfg.api_keys;
  for (const purpose of modelRoles) cfg.search[purpose] = gatherModel(purpose);
  cfg.search.vision = {provider:'disabled'}; cfg.search.llm = {provider:'disabled'};
  if (cfg.role === 'standalone') cfg.devices.filter(d => d.transport === 'ssh').forEach(d => d.enabled = false);
  cfg.devices.forEach(d => { d.roots = d.roots.map(v => v.trim()).filter(Boolean); d.exclude = d.exclude.map(v => v.trim()).filter(Boolean); d.analysis_roots = (d.analysis_roots || []).filter(p => d.roots.includes(p)); });
  return cfg;
}
function renderDevices() {
  $('devices').replaceChildren(); const shared = cfg.role === 'coordinator'; $('sshGuide').hidden = !shared;
  document.querySelectorAll('[data-step]').forEach(e => { e.hidden = cfg.role === 'client' && !['0','4'].includes(e.dataset.step); e.textContent = cfg.role === 'client' && e.dataset.step === '4' ? '2 · Review' : ['1 · Layout','2 · Devices','3 · Folders','4 · Search','5 · Review'][Number(e.dataset.step)]; });
  const viewer = cfg.role === 'client';
  $('hostSettings').hidden = viewer; $('viewerSettings').hidden = !viewer;
  $('hostSettings').querySelectorAll('input,select').forEach(e => e.disabled = viewer);
  $('modelLocation').textContent = viewer ? 'This viewer uses the main PC’s index and built-in models.' : 'EmbeddingGemma and Laya run inside the engine on this PC. Source devices collect selected files over SSH.';
  $('sshOptions').hidden = !shared; $('addDevice').hidden = !shared;
  $('deviceHelp').textContent = viewer ? 'This viewer connects to the main PC address. Folder choices are managed on the main PC.' : 'Each source has its own folder selection. SSH sources need no models; this PC runs native inference.';
  for (const d of cfg.devices) {
    if (cfg.role === 'client' || (!shared && d.transport === 'ssh')) continue;
    const card = element('div','','deviceCard'); const fields = element('div','','two');
    fields.append(field('Device name',d.label,v => { d.label = v; renderFolderChoices(); }).wrapper);
    const advanced = element('details'); advanced.append(element('summary','Connection details')); const id = field('Stable device ID',d.id,v => { d.id = v; renderFolderChoices(); }); id.input.readOnly = cfg.configured; advanced.append(id.wrapper);
    if (d.transport === 'ssh') advanced.append(field('Python executable on source',d.python || 'auto',v => d.python = v).wrapper);
    if (d.transport === 'ssh') {
      const alias = field('SSH Host alias',d.ssh || '',v => d.ssh = v); alias.input.setAttribute('list','aliases'); fields.append(alias.wrapper);
      const label = element('label','Operating system'); const select = document.createElement('select'); ['auto','windows','linux','macos'].forEach(p => select.append(new Option(p,p))); select.value = d.platform; select.onchange = () => d.platform = select.value; label.append(select); fields.append(label);
    } else card.append(element('p','This computer · ' + d.platform,'hint'));
    card.append(fields,advanced);
    const controls = element('div','','deviceActions'); const enabled = element('label','Enabled','check'); const check = document.createElement('input'); check.type = 'checkbox'; check.checked = d.enabled; check.onchange = () => { d.enabled = check.checked; renderFolderChoices(); }; enabled.prepend(check); controls.append(enabled);
    const probe = element('button','Test connection'); probe.type = 'button'; const message = element('span','','probeState');
    probe.onclick = async () => { probe.disabled = true; message.textContent = 'Checking SSH and Python…'; message.classList.remove('error'); try { const result = await api('/api/device-probe','POST',{config:gather(),device:d.id}); message.textContent = `${result.hostname} · ${result.platform} · Python ${result.python}`; } catch (e) { message.textContent = e.message; message.classList.add('error'); } finally { probe.disabled = false; } }; controls.append(probe);
    if (d.transport === 'ssh') { const remove = element('button','Remove'); remove.type = 'button'; remove.onclick = () => { cfg.devices.splice(cfg.devices.indexOf(d),1); renderDevices(); renderFolderChoices(); markDraft(); }; controls.append(remove); }
    card.append(controls,message); $('devices').append(card);
  }
}
function addDevice(alias = '') { let i = 1; while (cfg.devices.some(d => d.id === `device-${i}`)) i++; cfg.devices.push({id:`device-${i}`,label:alias || `Device ${i}`,transport:'ssh',ssh:alias,python:'auto',platform:'auto',enabled:true,roots:[],exclude:[]}); renderDevices(); renderFolderChoices(); markDraft(); }
$('addDevice').onclick = () => addDevice();
$('addAlias').onclick = () => { const alias = $('sshAlias').value; if (!alias) return; if (cfg.devices.some(d => d.transport === 'ssh' && d.ssh === alias)) { $('aliasState').textContent = 'That alias is already in your devices.'; return; } addDevice(alias); };
function renderFolderChoices() {
  const previous = $('folderDevice').value; $('folderDevice').replaceChildren();
  for (const [i,d] of cfg.devices.entries()) if (cfg.role !== 'client' && (cfg.role === 'coordinator' || d.transport === 'local')) $('folderDevice').append(new Option(d.label,String(i)));
  if ([...$('folderDevice').options].some(o => o.value === previous)) $('folderDevice').value = previous;
  renderFolders();
}
function renderFolders() {
  $('folderEditor').replaceChildren(); const d = cfg.devices[Number($('folderDevice').value)];
  if (cfg.role === 'client' || !d) { $('folderEditor').append(element('p','Folders are managed on your coordinator.')); return; }
  for (const [key, title, action] of [['roots','Included folders','include'],['exclude','Excluded folders and patterns','exclude']]) {
    const section = element('section','','folderGroup'); section.append(element('h3',title));
    if (!d[key].length) section.append(element('p',key === 'roots' ? 'Choose the folders you want to search.' : 'Choose subfolders to leave out, or enter a name or pattern.','hint'));
    for (const [i,path] of d[key].entries()) {
      const row = element('div','','folderRow'); const input = document.createElement('input'); input.value = path;
      input.setAttribute('aria-label', key === 'roots' ? 'Included folder' : 'Excluded folder or pattern');
      input.placeholder = key === 'exclude' ? 'Backups, private/* or an absolute folder path' : (d.platform === 'windows' ? 'C:\\Users\\you\\Documents' : '~/Documents');
      input.oninput = () => { if (key === 'roots') d.analysis_roots = (d.analysis_roots || []).map(p => p === d[key][i] ? input.value : p); d[key][i] = input.value; };
      const browse = element('button','Browse…'); browse.type = 'button'; browse.setAttribute('aria-label',`Change ${action} folder ${i + 1}`);
      browse.onclick = () => openFolderBrowser(d,key,i);
      const remove = element('button','Remove'); remove.type = 'button'; remove.setAttribute('aria-label',`Remove ${action} folder ${i + 1}`);
      remove.onclick = () => { d[key].splice(i,1); renderFolders(); markDraft(); }; row.append(input,browse,remove); section.append(row);
      if (key === 'roots') section.append(element('p','Choose this folder’s indexing schedule in Crawling & learning after saving.','hint'));
    }
    const actions = element('div','','folderActions');
    const browse = element('button',key === 'roots' ? '+ Browse to include' : '+ Browse to exclude'); browse.type = 'button'; browse.classList.toggle('primary',key === 'roots');
    browse.onclick = () => openFolderBrowser(d,key);
    const manual = element('button',key === 'roots' ? 'Enter a path' : 'Enter a pattern or path'); manual.type = 'button';
    manual.onclick = () => { d[key].push(''); renderFolders(); $('folderEditor').querySelectorAll(key === 'roots' ? '[aria-label="Included folder"]' : '[aria-label="Excluded folder or pattern"]')[d[key].length - 1]?.focus(); };
    actions.append(browse,manual); section.append(actions); $('folderEditor').append(section);
  }
  if (!d.enabled) $('folderEditor').prepend(element('p','This device is disabled. Enable it in Devices to include these folders in the next scan.','weak'));
  $('folderEditor').append(element('p','Hidden folders, symbolic links, build outputs and common credential markers are always excluded. Changes apply when you save settings.','hint'));
  $('folderEditor').append(element('p','Selected text is cached and indexed locally with EmbeddingGemma. Laya checks a short list when you search.','hint'));
}

function folderKey(path, platform) { return platform === 'windows' ? path.replaceAll('\\','/').replace(/\/+$/,'').toLowerCase() : path.replace(/\/+$/,'') || '/'; }
function folderButton(label, action, cls = '') { const button = element('button',label,cls); button.type = 'button'; button.onclick = action; return button; }
function renderFolderSelection() {
  if (!folderPicker) return;
  const {chosen, view, key, loading} = folderPicker, verb = key === 'roots' ? 'Include' : 'Exclude';
  $('chooseFolders').textContent = chosen.size ? `${verb} ${chosen.size} folder${chosen.size === 1 ? '' : 's'}` : `${verb} this folder`;
  $('chooseFolders').disabled = !chosen.size && (!view || loading || folderPicker.failed);
  $('folderSelection').textContent = chosen.size ? [...chosen.values()].join(' · ') : (view ? `Current folder: ${view.path}` : 'Choose a location to start browsing.');
  $('folderSelection').title = $('folderSelection').textContent;
}
function renderFolderLocations() {
  const {device,view} = folderPicker; $('folderPlaces').replaceChildren(element('h3',device.transport === 'ssh' ? 'On this SSH device' : 'On this PC'));
  for (const place of view?.places || [{label:'Home',path:''}]) {
    const button = folderButton(place.label,() => loadFolders(place.path), 'folderPlace'); button.title = place.path;
    if (view && folderKey(place.path,view.platform) === folderKey(view.path,view.platform)) button.setAttribute('aria-current','location');
    $('folderPlaces').append(button);
  }
  const roots = device.roots.filter(Boolean);
  if (roots.length) $('folderPlaces').append(element('h3','Included locations'));
  for (const path of roots) { const button = folderButton(path,() => loadFolders(path),'folderPlace path'); button.title = path; $('folderPlaces').append(button); }
}
async function openFolderBrowser(device, key, replaceIndex = null) {
  folderPicker = {device,key,replaceIndex,chosen:new Map(),history:[],historyIndex:-1,view:null,loading:false,failed:false};
  $('folderBrowserTitle').textContent = key === 'roots' ? 'Include folders' : 'Exclude folders';
  $('folderBrowserDevice').textContent = `${device.label} · ${device.transport === 'ssh' ? 'Connected through SSH' : 'This PC'}`;
  $('folderBrowserError').textContent = ''; $('folderFilter').value = ''; $('folderAddress').value = ''; $('folderList').replaceChildren(); $('folderBreadcrumbs').replaceChildren();
  $('folderCount').textContent = ''; $('folderPageBack').hidden = true; $('folderPageNext').hidden = true;
  renderFolderLocations(); renderFolderSelection(); $('folderBrowser').showModal();
  const path = replaceIndex !== null ? device[key][replaceIndex] : device.roots.find(Boolean);
  const absolute = path && /^(~([/\\]|$)|\/|[a-z]:[/\\]|\\\\)/i.test(path);
  await loadFolders(absolute ? path : '');
}
async function loadFolders(path, {offset = 0, filter = '', historyIndex = null, preserveFilter = false} = {}) {
  if (!folderPicker) return;
  clearTimeout(folderFilterTimer);
  const picker = folderPicker, request = ++folderRequest; picker.loading = true; picker.failed = false;
  if (!preserveFilter) $('folderFilter').value = filter;
  $('folderBrowserError').textContent = ''; $('folderList').setAttribute('aria-busy','true'); $('folderList').replaceChildren(element('p','Loading folders…','folderEmpty'));
  $('folderBack').disabled = true; $('folderForward').disabled = true; $('folderUp').disabled = true; $('folderPageBack').hidden = true; $('folderPageNext').hidden = true;
  renderFolderSelection();
  try {
    const data = await api('/api/folders','POST',{device:picker.device,ssh_config:$('sshConfig').value.trim(),path,query:filter,offset});
    if (request !== folderRequest || picker !== folderPicker) return;
    picker.view = data;
    if (historyIndex !== null) picker.historyIndex = historyIndex;
    else if (picker.history[picker.historyIndex] !== data.path) { picker.history.splice(picker.historyIndex + 1); picker.history.push(data.path); picker.historyIndex = picker.history.length - 1; }
    $('folderAddress').value = data.path; $('folderBreadcrumbs').replaceChildren();
    for (const crumb of data.breadcrumbs) { const button = folderButton(crumb.label,() => loadFolders(crumb.path)); button.title = crumb.path; if (crumb.path === data.path) button.setAttribute('aria-current','location'); $('folderBreadcrumbs').append(button); }
    renderFolderLocations(); $('folderList').replaceChildren();
    if (!data.folders.length) $('folderList').append(element('p',filter ? 'No matching subfolders. Try another filter.' : 'No browsable subfolders. You can select this folder.','folderEmpty'));
    for (const folder of data.folders) {
      const row = element('div','','folderItem'); const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.setAttribute('aria-label',`Select folder ${folder.name}`);
      const key = folderKey(folder.path,data.platform); checkbox.checked = picker.chosen.has(key);
      checkbox.onchange = () => { checkbox.checked ? picker.chosen.set(key,folder.path) : picker.chosen.delete(key); renderFolderSelection(); };
      const button = folderButton('',() => loadFolders(folder.path),'folderOpen'); button.setAttribute('aria-label',`Open folder ${folder.name}`); button.title = folder.path;
      button.append(element('span','▰','folderIcon'),element('span',folder.name,'folderName'),element('span','›','folderChevron')); row.append(checkbox,button); $('folderList').append(row);
    }
    $('folderCount').textContent = `${data.total} folder${data.total === 1 ? '' : 's'}` + (data.total > data.folders.length ? ` · ${data.offset + 1}–${data.offset + data.folders.length} shown` : '');
    $('folderPageBack').hidden = data.offset === 0; $('folderPageNext').hidden = data.next_offset === null;
  } catch (e) {
    if (request !== folderRequest || picker !== folderPicker) return;
    picker.failed = true; $('folderBrowserError').textContent = e.message; $('folderList').replaceChildren(element('p','Choose another location or enter a folder path.','folderEmpty')); $('folderCount').textContent = '';
  } finally {
    if (request === folderRequest && picker === folderPicker) {
      picker.loading = false; $('folderList').setAttribute('aria-busy','false');
      $('folderBack').disabled = picker.historyIndex <= 0; $('folderForward').disabled = picker.historyIndex >= picker.history.length - 1;
      $('folderUp').disabled = !picker.view?.parent; renderFolderSelection();
    }
  }
}
function closeFolderBrowser() { ++folderRequest; clearTimeout(folderFilterTimer); folderPicker = null; $('folderBrowser').close(); $('folderDevice').focus(); }
$('closeFolderBrowser').onclick = closeFolderBrowser; $('cancelFolderBrowser').onclick = closeFolderBrowser;
$('folderBrowser').oncancel = e => { e.preventDefault(); closeFolderBrowser(); };
$('folderAddressForm').onsubmit = e => { e.preventDefault(); loadFolders($('folderAddress').value.trim()); };
$('folderBack').onclick = () => { const i = folderPicker.historyIndex - 1; loadFolders(folderPicker.history[i],{historyIndex:i}); };
$('folderForward').onclick = () => { const i = folderPicker.historyIndex + 1; loadFolders(folderPicker.history[i],{historyIndex:i}); };
$('folderUp').onclick = () => loadFolders(folderPicker.view.parent);
$('folderFilter').oninput = () => { clearTimeout(folderFilterTimer); folderFilterTimer = setTimeout(() => { if (folderPicker?.view) loadFolders(folderPicker.view.path,{filter:$('folderFilter').value,preserveFilter:true}); },350); };
$('folderFilter').onkeydown = e => { if (e.key === 'Enter') { e.preventDefault(); clearTimeout(folderFilterTimer); if (folderPicker?.view) loadFolders(folderPicker.view.path,{filter:$('folderFilter').value,preserveFilter:true}); } };
$('folderPageNext').onclick = () => loadFolders(folderPicker.view.path,{offset:folderPicker.view.next_offset,filter:$('folderFilter').value,preserveFilter:true});
$('folderPageBack').onclick = () => loadFolders(folderPicker.view.path,{offset:Math.max(0,folderPicker.view.offset - 200),filter:$('folderFilter').value,preserveFilter:true});
$('chooseFolders').onclick = () => {
  const {device,key,replaceIndex,chosen,view} = folderPicker, paths = chosen.size ? [...chosen.values()] : [view.path];
  const existing = device[key].filter((v,i) => v.trim() && i !== replaceIndex), additions = paths.filter(v => !existing.some(p => folderKey(p,view.platform) === folderKey(v,view.platform)));
  if (existing.length + additions.length > 100) { $('folderBrowserError').textContent = 'A maximum of 100 folders or patterns can be selected for each list.'; return; }
  if (replaceIndex !== null) device[key].splice(replaceIndex,1,...additions); else device[key].push(...additions);
  renderFolders(); closeFolderBrowser(); markDraft();
};
$('folderDevice').onchange = renderFolders;
document.querySelectorAll('[name=role]').forEach(e => e.onchange = () => { gather(); renderDevices(); renderFolderChoices(); });
$('importSSH').onclick = async () => { $('importSSH').disabled = true; $('aliasState').textContent = 'Reading your SSH config…'; try { const data = await api('/api/ssh-aliases','POST',{path:$('sshConfig').value.trim()}); $('aliases').replaceChildren(...data.aliases.map(a => new Option(a,a))); $('sshAlias').replaceChildren(...data.aliases.map(a => new Option(a,a))); $('aliasChoices').hidden = !data.aliases.length; $('aliasState').textContent = data.aliases.length ? `${data.aliases.length} aliases found` + (data.ssh_available ? '' : ' · OpenSSH client missing') : 'No named aliases found. Add a Host entry to your SSH config, then read it again.'; } catch (e) { $('aliasState').textContent = e.message; } finally { $('importSSH').disabled = false; } };
async function checkReadiness() {
  $('checkReadiness').disabled = true; $('searchReadiness').replaceChildren(element('p','Checking this PC…'));
  try { const draft = JSON.stringify(gather()); const plan = await api('/api/plan','POST',JSON.parse(draft)); if (draft !== JSON.stringify(gather())) return; const caps = plan.capabilities; $('searchReadiness').replaceChildren();
    const embedding = caps.embedding, relevance = caps.relevance;
    for (const [title,ready,text] of [['Word search',true,'Filename, document text and code search included.'],['Meaning search',caps.semantic,embedding.detail],['Laya relevance',relevance.available,relevance.detail]]) { const card = element('div','','readinessCard'); card.dataset.ready = ready; card.append(element('strong',title),element('span',ready ? 'Ready' : 'Setup','badge'),element('p',text)); $('searchReadiness').append(card); }
  } catch (e) { $('searchReadiness').replaceChildren(element('p',e.message,'error')); } finally { $('checkReadiness').disabled = false; }
}
$('checkReadiness').onclick = checkReadiness;
function stepOrder() { return cfg.role === 'client' ? [0,4] : [0,1,2,3,4]; }
async function showStep(value) {
  const request = ++planRequest; step = value; $('setupError').textContent = ''; $('reloadSettings').hidden = true;
  document.querySelectorAll('[data-panel]').forEach(e => e.hidden = Number(e.dataset.panel) !== step);
  document.querySelectorAll('[data-step]').forEach(e => { const active = Number(e.dataset.step) === step; e.classList.toggle('active',active); active ? e.setAttribute('aria-current','step') : e.removeAttribute('aria-current'); });
  $('previousStep').hidden = step === 0; $('nextStep').hidden = step === 4; $('saveSetup').hidden = step !== 4;
  if (step === 3) checkReadiness();
  if (step === 4) { $('plan').replaceChildren(element('p','Checking your setup…')); $('saveSetup').disabled = true; try { const plan = await api('/api/plan','POST',gather()); if (request !== planRequest) return; renderPlan(plan); $('saveSetup').disabled = false; } catch (e) { if (request !== planRequest) return; $('setupError').textContent = e.message; $('plan').replaceChildren(element('p','Resolve the settings above before saving.')); } }
}
function renderPlan(plan) {
  $('plan').replaceChildren(); const card = element('div','','planCard'); card.append(element('h3',({standalone:'Search this computer',coordinator:'This is your main PC',client:'Connect to your main PC as a search viewer'})[plan.role]));
  const steps = element('ul'); plan.steps.forEach(s => steps.append(element('li',s))); card.append(steps,element('p','Private storage: ' + plan.storage,'path')); $('plan').append(card);
  for (const d of plan.devices) { if (!d.enabled || plan.role === 'client') continue; const device = element('div','','planCard'); device.append(element('h3',d.label),element('p',d.installation)); const paths = element('ul'); d.roots.forEach(p => paths.append(element('li','Include: ' + p,'path'))); d.exclude.forEach(p => paths.append(element('li','Exclude: ' + p,'path'))); if (!d.roots.length) paths.append(element('li','No folders selected')); device.append(paths); $('plan').append(device); }
  plan.warnings.forEach(w => $('plan').append(element('p',w,'weak')));
  const relevance = plan.capabilities.relevance;
  if (relevance.enabled && plan.role !== 'client') $('plan').append(element('p',relevance.label + ' relevance ordering: ' + relevance.state.replaceAll('_',' ') + '. Exact searches keep retrieval order.','hint'));
  for (const item of plan.api_data || []) $('plan').append(element('p',`${item.purpose}: ${item.model} at ${item.endpoint} (${item.hosting}). Sends ${item.sent}.`,'architectureNote'));
}
document.querySelectorAll('[data-step]').forEach(e => e.onclick = () => showStep(Number(e.dataset.step)));
$('previousStep').onclick = () => { const order = stepOrder(); showStep(order[Math.max(0,order.indexOf(step)-1)]); }; $('nextStep').onclick = () => { gather(); const order = stepOrder(); showStep(order[Math.min(order.length-1,order.indexOf(step)+1)]); };
$('setupForm').onsubmit = async e => { e.preventDefault(); $('saveSetup').disabled = true; try { const data = await api('/api/config','POST',gather(),configRevision); cfg = data.config; configRevision = data.revision; fillModelForms(); savedSnapshot = JSON.stringify(gather()); $('steps').hidden = true; $('draftState').textContent = 'Saved on this computer.'; $('setupForm').hidden = true; $('setupDone').hidden = false; $('sync').hidden = cfg.role === 'client'; $('openCoordinator').hidden = cfg.role !== 'client'; $('openCoordinator').href = cfg.endpoint; $('openCoordinator').style.removeProperty('display'); $('doneNote').textContent = cfg.role === 'client' ? 'Client address saved. Open the coordinator and enter its pairing token to connect.' : cfg.devices.some(d => d.enabled && d.roots.length) ? 'Your folders are ready. Index them now, or return to search and scan later.' : 'No folders selected yet. Return to settings to choose folders before indexing.'; if (bridge || window.webkit?.messageHandlers?.searchx) { await native('appearance',cfg.appearance).catch(() => {}); if (cfg.role === 'client') await native('endpoint',{url:cfg.endpoint}); } await status(); } catch (e) { $('setupError').textContent = e.message; $('reloadSettings').hidden = !(e.status === 409 || /changed|reload/i.test(e.message)); } finally { $('saveSetup').disabled = false; } };
$('sync').onclick = async () => { try { const result = await api('/api/sync','POST',{}); $('indexState').textContent = result.state === 'started' ? 'Starting your scan…' : 'A scan is already running.'; await status(); } catch (e) { $('indexState').textContent = e.message; } };
$('showIndex').onclick = async () => { $('indexDialog').showModal(); await status(); };
$('closeIndex').onclick = () => $('indexDialog').close();
$('scanNow').onclick = async () => { $('scanNow').disabled = true; try { await api('/api/sync','POST',{}); await status(); } catch (e) { $('scanSummary').textContent = e.message; $('scanNow').disabled = false; } };
$('rebuildMeaning').onclick = async () => { $('rebuildMeaning').disabled = true; try { await api('/api/sync','POST',{index_only:true,force_embeddings:latestStatus?.embedding?.provider !== 'native'}); $('scanSummary').textContent = 'Rebuilding meaning search from collected files with your saved embedding provider…'; await status(); } catch (e) { $('scanSummary').textContent = e.message; } finally { $('rebuildMeaning').disabled = Boolean(latestStatus?.syncing); } };
function finishSetup() { $('discardDialog').close(); $('wizard').hidden = true; $('searchPane').hidden = false; $('query').focus(); window.searchxWorkspace?.restore(); }
function isDirty() { return cfg && savedSnapshot && JSON.stringify(gather()) !== savedSnapshot; }
function markDraft() { if (cfg) $('draftState').textContent = isDirty() ? 'Unsaved changes · save at Review.' : 'Changes are saved at the final step.'; }
function closeSetup() { if (!$('setupForm').hidden && isDirty()) { $('discardDialog').showModal(); return; } finishSetup(); }
$('setupForm').addEventListener('input',markDraft); $('setupForm').addEventListener('change',markDraft);
$('keepEditing').onclick = () => $('discardDialog').close(); $('discardChanges').onclick = () => { savedSnapshot = ''; finishSetup(); };
$('reloadSettings').onclick = openSetup;
window.addEventListener('beforeunload',e => { if (!$('wizard').hidden && !$('setupForm').hidden && isDirty()) { e.preventDefault(); e.returnValue = ''; } });
$('closeSetup').onclick = closeSetup; $('returnSearch').onclick = closeSetup; $('settings').onclick = openSetup; $('welcomeSetup').onclick = openSetup;
for (const action of ['hide','minimize','maximize']) $(''+action).onclick = () => native(action).catch(() => {});
$('resize').onpointerdown = () => native('resize').catch(() => {}); $('drag').onpointerdown = e => { if (!e.target.closest('button')) native('drag').catch(() => {}); };
document.addEventListener('keydown',e => { if (e.defaultPrevented || document.querySelector('dialog[open]')) return; if (e.key === 'Escape') { if ($('indexDialog').open) { e.preventDefault(); $('indexDialog').close(); return; } if ($('discardDialog').open) { e.preventDefault(); $('discardDialog').close(); return; } if ($('folderBrowser').open) { e.preventDefault(); closeFolderBrowser(); return; } if (!$('wizard').hidden) closeSetup(); else if (!$('preview').hidden) back(); else if (!$('searchPane').hidden) native('hide').catch(() => {}); } if (['ArrowDown','ArrowUp'].includes(e.key) && !document.querySelector('dialog[open]') && hits.length && !$('searchPane').hidden && ['INPUT','TEXTAREA','SELECT'].includes(e.target.tagName) && e.target !== $('query')) return; if (['ArrowDown','ArrowUp'].includes(e.key) && !document.querySelector('dialog[open]') && hits.length && !$('searchPane').hidden && $('preview').hidden) { e.preventDefault(); selected = Math.max(0,Math.min(hits.length-1,selected+(e.key==='ArrowDown'?1:-1))); document.querySelectorAll('.hit').forEach((r,i) => r.classList.toggle('selected',i===selected)); document.querySelectorAll('.hit')[selected]?.scrollIntoView({block:'nearest'}); } });
window.searchxFocus = () => { $('query').focus(); status(); };
async function init() { const desktop = !!(bridge || window.webkit?.messageHandlers?.searchx || window.chrome?.webview); document.body.classList.toggle('desktop',desktop); if (!desktop) for (const id of ['hide','minimize','maximize','resize']) $(id).hidden = true; const hash = new URLSearchParams(location.hash.slice(1)); if (hash.has('token')) { const token = hash.get('token'); history.replaceState(null,'',location.pathname); try { await signIn(token); } catch (e) { showLogin(); $('loginError').textContent = e.message; } } else await status(true); }
if (window.qt) { const script = document.createElement('script'); script.src = 'qrc:///qtwebchannel/qwebchannel.js'; script.onload = () => new QWebChannel(qt.webChannelTransport,c => { bridge = c.objects.searchx; init(); }); document.head.append(script); } else init();
