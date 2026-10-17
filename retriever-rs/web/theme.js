'use strict';
(() => {
  const system=matchMedia('(prefers-color-scheme:dark)');
  let preference='dark';try{preference=localStorage.getItem('retriever-theme')||localStorage.getItem('golden-retriever-theme')||'dark';}catch{}
  function apply(){document.documentElement.dataset.theme=preference==='system'?(system.matches?'dark':'light'):preference==='light'?'light':'dark';const button=document.getElementById('themeToggle');if(button){button.title=document.documentElement.dataset.theme==='dark'?'Use light appearance':'Use dark appearance';button.setAttribute('aria-label',button.title);}dispatchEvent(new Event('retrieverThemeChanged'));}
  window.retrieverTheme={preference:()=>preference,set(value){preference=['light','dark','system'].includes(value)?value:'dark';try{localStorage.setItem('retriever-theme',preference);}catch{}apply();}};
  system.addEventListener('change',()=>{if(preference==='system')apply();});apply();
  addEventListener('DOMContentLoaded',()=>{const button=document.getElementById('themeToggle');if(button)button.onclick=()=>window.retrieverTheme.set(document.documentElement.dataset.theme==='dark'?'light':'dark');apply();});
})();
