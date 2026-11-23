/* thème AVANT le premier rendu (anti-flash) : mode sombre par défaut, clair uniquement si choisi dans ⚙ Réglages */
try{const t=localStorage.getItem('mcc.theme')||localStorage.getItem('mcc-theme');document.documentElement.dataset.theme=t==='light'?'light':'dark'}catch(e){document.documentElement.dataset.theme='dark'}
