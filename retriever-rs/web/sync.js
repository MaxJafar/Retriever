'use strict';
(() => {
  let overview=null,syncPlan=null;
  const post=(route,body)=>api('/api/workspace/'+route,'POST',body);
  const label=id=>overview?.devices.find(d=>d.id===id)?.label || id;
  const fail=(id,error)=>{$(id).textContent=error?.message || String(error);};
  const action=(title,handler)=>{const button=element('button',title);button.type='button';button.onclick=handler;return button;};
  async function refresh(){overview=await api('/api/workspace');render();}
  function locations(side){
    const device=overview.devices.find(d=>d.id===$(side+'SyncDevice').value);
    $(side+'SyncPath').value=device?.roots[0] || '';
    $(side+'SyncRoots').replaceChildren();for(const root of device?.roots || []){const option=element('option');option.value=root;$(side+'SyncRoots').append(option);}
  }
  function render(){
    $('syncPairs').replaceChildren();
    for(const pair of overview.sync_pairs || []){
      const card=element('div','','syncPair');card.append(element('h3',pair.name));
      for(const side of ['left','right']){const loc=pair.locations[side];card.append(element('p',`${label(loc.device)} · ${loc.path}`,'path'));}
      card.append(element('p',pair.last_sync?'Last synced '+new Date(pair.last_sync*1000).toLocaleString():'Ready for its first reviewed sync','hint'));
      const controls=element('div','','folderActions');
      controls.append(action('Preview sync',async()=>{try{$('syncError').textContent='Comparing selected folders…';const value=await post('sync-plan',{id:pair.id});$('syncError').textContent='';show(value);}catch(e){fail('syncError',e);}}));
      controls.append(action('Unlink pair',async()=>{try{await post('sync-pair',{id:pair.id,remove:true});await refresh();}catch(e){fail('syncError',e);}}));card.append(controls);$('syncPairs').append(card);
    }
    if(!overview.sync_pairs?.length)$('syncPairs').append(element('p','Connect two included folders to keep their files together. Nothing syncs until you review and apply.','hint'));
    for(const side of ['left','right']){
      const selected=$(side+'SyncDevice').value;$(side+'SyncDevice').replaceChildren();
      for(const device of overview.devices){const option=element('option',device.label);option.value=device.id;$(side+'SyncDevice').append(option);}
      if(selected)$(side+'SyncDevice').value=selected;
      if(!$(side+'SyncPath').value)locations(side);
    }
  }
  function show(value){
    syncPlan=value;$('syncPlanTitle').textContent='Review sync · '+value.pair_name;$('syncPlanItems').replaceChildren();$('syncPlanError').textContent='';
    for(const dir of value.directories){const row=element('div','Create folder · '+label(value.locations[dir.side].device)+' / '+dir.relative,'syncItem');$('syncPlanItems').append(row);}
    for(const item of value.items){const row=element('div',item.relative,'syncItem');row.append(element('small',item.reason+' · '+label(value.locations[item.origin].device)+' → '+label(value.locations[item.target].device)));row.append(element('small',item.destination));if(item.previous_version)row.append(element('small','Previous file kept at '+item.previous_version));$('syncPlanItems').append(row);}
    for(const note of value.notices){const row=element('div',note.path,'syncItem syncConflict');row.append(element('small',note.message));$('syncPlanItems').append(row);}
    const skipped=Object.values(value.snapshots).reduce((sum,s)=>sum+s.skipped,0);
    $('syncPlanSummary').textContent=`${value.items.length} file transfers · ${value.directories.length} new folders · ${value.notices.length} notices${skipped?' · '+skipped+' excluded entries':''}`;
    if(!value.items.length && !value.directories.length)$('syncPlanItems').prepend(element('p','No file changes are needed. Confirm to record this comparison as the baseline.','hint'));
    $('applySyncPlan').disabled=false;$('applySyncPlan').textContent=value.items.length || value.directories.length?'Apply reviewed sync':'Confirm baseline';$('syncPlanDialog').showModal();
  }
  $('folderSync').onclick=async()=>{try{$('syncError').textContent='';await refresh();$('syncDialog').showModal();}catch(e){fail('workspaceMessage',e);}};
  $('closeSync').onclick=()=>$('syncDialog').close();$('closeSyncPlan').onclick=()=>$('syncPlanDialog').close();
  for(const side of ['left','right'])$(side+'SyncDevice').onchange=()=>locations(side);
  $('createSyncPair').onsubmit=async event=>{event.preventDefault();try{await post('sync-pair',{name:$('syncPairName').value,left:{device:$('leftSyncDevice').value,path:$('leftSyncPath').value},right:{device:$('rightSyncDevice').value,path:$('rightSyncPath').value}});$('syncPairName').value='';await refresh();$('syncError').textContent='Pair saved. Preview it before syncing.';}catch(e){fail('syncError',e);}};
  $('applySyncPlan').onclick=async()=>{
    if(!syncPlan)return;$('applySyncPlan').disabled=true;$('closeSyncPlan').disabled=true;
    try{let result=await post('apply',{id:syncPlan.id});while(['queued','applying'].includes(result.state)){await new Promise(resolve=>setTimeout(resolve,500));result=await api('/api/workspace/operation/'+result.id);$('syncPlanSummary').textContent=result.transfer?.state==='copying'?`Copying file ${result.transfer.file} of ${result.transfer.files}…`:'Applying reviewed sync…';}
      if(result.state!=='applied')throw new Error(result.receipt?.error || 'Sync needs inspection in Activity. Inspect both folders before retrying.');
      $('syncPlanDialog').close();await refresh();$('syncError').textContent=`Sync complete. ${result.receipt.items?.length || 0} files copied; previous versions and deletions preserved.${result.receipt.index_required?' Live Files is current; meaning search updates at the next index.':''}`;
      window.dispatchEvent(new Event('goldenFilesChanged'));
    }catch(e){fail('syncPlanError',e);}finally{$('closeSyncPlan').disabled=false;}
  };
  function installation(){
    const os=$('clientPlatform').value,quote=value=>"'"+value.replaceAll("'",os==='windows'?"''":"'\\''")+"'";
    const flags=['--role','viewer','--desktop'];
    const alias=$('clientSSH').value.trim();
    if(alias){flags.push('--main-ssh',quote(alias),'--local-port','18769','--remote-port','8769');if($('clientSSHConfig').value.trim())flags.push('--ssh-config',quote($('clientSSHConfig').value.trim()));}
    else flags.push('--endpoint',quote($('clientEndpoint').value.trim() || 'http://127.0.0.1:18769'));
    if($('clientDevice').value)flags.push('--device-id',quote($('clientDevice').value));
    if($('clientAppPath').value.trim())flags.push('--install-dir',quote($('clientAppPath').value.trim()));
    if($('clientDataPath').value.trim())flags.push('--state',quote($('clientDataPath').value.trim()));
    $('clientInstallCommand').textContent=(os==='windows'?'python':'python3')+' install.py '+flags.join(' ');
    const defaults={windows:['%LOCALAPPDATA%\\Programs\\Retriever','%LOCALAPPDATA%\\Retriever','Start menu → Retriever'],macos:['~/Library/Application Support/Retriever/app','~/Library/Application Support/Retriever','~/Applications/Retriever.app'],linux:['~/.local/share/golden-retriever/app','~/.local/share/golden-retriever','Applications menu → Retriever']};
    const [app,data,entry]=defaults[os];$('clientDefaultPaths').textContent=`Default app: ${app}\nDefault private data: ${data}\nLaunch: ${entry}`;
    $('clientInstallNote').textContent=alias?'The desktop app opens and closes its strict SSH tunnel for you. The main PC needs SSH key access already configured. Pair in the app with the main PC token.':'A loopback endpoint needs an existing SSH tunnel. Add its main-PC SSH alias above to let the app manage it, or use a restricted HTTPS endpoint.';
  }
  for(const id of ['clientPlatform','clientSSH','clientSSHConfig','clientEndpoint','clientDevice','clientAppPath','clientDataPath'])$(id).addEventListener('input',installation);
  $('clientInstallGuide').onclick=async()=>{try{overview=await api('/api/workspace');const placeholder=element('option','Choose this client’s device (optional)');placeholder.value='';$('clientDevice').replaceChildren(placeholder);for(const device of overview.devices){const option=element('option',device.label);option.value=device.id;$('clientDevice').append(option);}installation();$('clientsDialog').showModal();}catch(e){fail('workspaceMessage',e);}};
  $('closeClients').onclick=()=>$('clientsDialog').close();$('copyClientInstall').onclick=async()=>{try{const text=$('clientInstallCommand').textContent;await native('copy',{text}).catch(()=>navigator.clipboard.writeText(text));$('clientInstallNote').textContent='Installation command copied.';}catch(e){fail('clientInstallNote',e);}};
})();
