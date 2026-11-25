/* anti-flash thème & texte */
try{const d=document.documentElement.dataset,t=localStorage.getItem('mcc.theme')||localStorage.getItem('mcc-theme'),c=localStorage.getItem('mcc.textColor'),b=localStorage.getItem('mcc.textBright');d.theme=t==='light'?'light':'dark';if(c&&c!=='default')d.textColor=c;if(b&&b!=='100')d.textBright=b}catch(e){document.documentElement.dataset.theme='dark'}
