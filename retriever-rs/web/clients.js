'use strict';
(() => {
  let plan=null;
  const dialog=element('dialog','','workspaceDialog');dialog.id='remoteClientsDialog';dialog.setAttribute('aria-label','Remote client installation');
  const heading=element('div','','folderBrowserHeading');heading.append(element('h2','Install on another device'));const close=element('button','×');close.setAttribute('aria-label','Close remote installation');close.onclick=()=>dialog.close();heading.append(close);dialog.append(heading);
  dialog.append(element('p','Use a verified SSH connection over Tailscale or your private network. The main PC installs a model-free client, pairs it and maintains its private loopback connection. Files sync only after you review and apply a separate sync preview.'));
  const installed=element('div');dialog.append(installed);
  const form=element('form');form.id='remoteClientForm';
  function field(text,id,type='input'){const label=element('label',text),input=element(type);input.id=id;label.append(input);form.append(label);return input;}
  const device=field('SSH device','remoteClientDevice','select');
  const mode=field('Client interface','remoteClientMode','select');for(const [value,label] of [['gui','Desktop app'],['headless','Linux — no GUI']]){const option=element('option',label);option.value=value;mode.append(option);}
  const app=field('Application path (optional)','remoteClientApp');app.placeholder='Use the target’s default application path';
  const data=field('Private client data path (optional)','remoteClientData');data.placeholder='Use the target’s default data path';
  const port=field('Client loopback port','remoteClientPort');port.type='number';port.min=1024;port.max=65535;port.value=18769;
  const preview=element('button','Detect device & review installation','primary');preview.type='submit';form.append(preview);dialog.append(form);
  const review=element('div','','clientCard');review.hidden=true;dialog.append(review);
  const message=element('p');message.setAttribute('role','status');message.id='remoteClientMessage';dialog.append(message);document.body.append(dialog);
  async function refresh(){
    const [workspace,catalog]=await Promise.all([api('/api/workspace'),api('/api/clients')]);
    device.replaceChildren();for(const item of workspace.devices.filter(d=>d.transport==='ssh')){const option=element('option',item.label);option.value=item.id;device.append(option);}
    installed.replaceChildren();
    for(const client of catalog.clients){const card=element('div','','clientCard');card.append(element('h3',client.label+' · '+client.mode),element('p',client.connection+' · '+client.version));const check=element('button','Check installed client');check.onclick=async()=>{check.disabled=true;try{const result=await api('/api/clients/check','POST',{device:client.device});message.textContent=`Connected · ${result.files.toLocaleString()} indexed files · models stay on the main PC`;}catch(e){message.textContent=e.message;}finally{check.disabled=false;}};card.append(check);installed.append(card);}
    if(!catalog.installers.length)message.textContent='Register native installers on the main PC before deploying clients.';
    return catalog;
  }
  $('remoteInstallGuide').onclick=async()=>{try{message.textContent='';review.hidden=true;await refresh();dialog.showModal();}catch(e){$('workspaceMessage').textContent=e.message;}};
  form.onsubmit=async event=>{event.preventDefault();preview.disabled=true;message.textContent='Checking the SSH device…';review.hidden=true;
    try{plan=await api('/api/clients/preview','POST',{device:device.value,mode:mode.value,app_dir:app.value.trim(),state:data.value.trim(),port:Number(port.value)});
      review.replaceChildren(element('h3','Review installation · '+plan.label),element('p',`${plan.target.platform} · ${plan.target.arch} · ${plan.installer.version} · ${plan.installer.mode}`),element('p','App: '+plan.app_dir,'path'),element('p','Private data: '+plan.state,'path'),element('p','Connection: '+plan.profile.endpoint),element('p',plan.review));
      const apply=element('button','Install reviewed client','primary');apply.onclick=()=>install(apply);review.append(apply);review.hidden=false;message.textContent='Nothing has been installed yet. This review expires in ten minutes.';
    }catch(e){message.textContent=e.message;}finally{preview.disabled=false;}
  };
  async function install(button){
    button.disabled=true;close.disabled=true;preview.disabled=true;
    try{let job=await api('/api/clients/apply','POST',{id:plan.id});while(['queued','installing'].includes(job.state)){message.textContent=job.phase;await new Promise(resolve=>setTimeout(resolve,700));job=await api('/api/clients/operation/'+job.id);}
      if(job.state!=='installed')throw new Error(job.phase+' · Inspect this installation before retrying.');review.hidden=true;await refresh();message.textContent='Client installed and checked. All AI remains on the main PC; folder sync still requires review.';
    }catch(e){message.textContent=e.message;}finally{close.disabled=false;preview.disabled=false;}
  }
  $('clientInstallGuide').addEventListener('click',async()=>{try{const catalog=await api('/api/clients');$('nativeInstallers').replaceChildren();
    if(location.protocol==='file:'){$('nativeInstallers').append(element('p','Use Remote install for another connected device. Native installers can also be downloaded in the main PC browser.','hint'));return;}
    for(const item of catalog.installers.filter(i=>i.kind!=='payload')){const link=element('a',`${item.platform} · ${item.mode} · ${item.kind.toUpperCase()}`,'button');link.href='/api/installers/'+item.id;link.setAttribute('download',item.filename);$('nativeInstallers').append(link);}
    if(!catalog.installers.length)$('nativeInstallers').append(element('p','Native installers have not been registered on this main PC.','hint'));
  }catch(e){$('clientInstallNote').textContent=e.message;}});
})();
