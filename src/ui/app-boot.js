/* thème AVANT le premier rendu (anti-flash) : préférence ⚙ Réglages ou OS */
try{const t=localStorage.getItem('mcc.theme')||localStorage.getItem('mcc-theme');document.documentElement.dataset.theme=t==='light'||t==='dark'?t:(window.matchMedia&&window.matchMedia('(prefers-color-scheme: light)').matches?'light':'dark')}catch(e){}
