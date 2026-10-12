
(function(){
  var root=document.documentElement;
  function setTheme(name){
    root.classList.remove('light','dark');root.classList.add(name);
    var b=document.getElementById('theme');
    if(b){b.textContent=name==='dark'?'Light':'Dark';}
    try{localStorage.setItem('tg-report-theme',name);}catch(e){}
  }
  try{var saved=localStorage.getItem('tg-report-theme');if(saved)setTheme(saved);}catch(e){}
  var themeBtn=document.getElementById('theme');
  if(themeBtn)themeBtn.addEventListener('click',function(){
    setTheme(root.classList.contains('dark')?'light':'dark');
  });
  function toggle(id,cls){
    var b=document.getElementById(id);
    if(!b)return;
    b.addEventListener('click',function(){
      var on=root.classList.toggle(cls);
      b.setAttribute('aria-pressed',on?'true':'false');
    });
  }
  toggle('aliases','aliases');
  toggle('everyone','everyone');

  var tip=document.getElementById('tip');
  function show(el,x,y){
    var text=el.getAttribute('data-tip');
    if(!text)return;
    tip.textContent=text;tip.classList.add('on');
    tip.style.left=x+'px';tip.style.top=y+'px';
  }
  document.addEventListener('mousemove',function(e){
    var el=e.target.closest?e.target.closest('[data-tip]'):null;
    if(el){show(el,e.clientX,e.clientY);}else{tip.classList.remove('on');}
  });
  document.addEventListener('focusin',function(e){
    var el=e.target.closest?e.target.closest('[data-tip]'):null;
    if(!el){tip.classList.remove('on');return;}
    var r=el.getBoundingClientRect();show(el,r.left+r.width/2,r.top);
  });

  function selectEvent(id){
    document.querySelectorAll('.ev.on,.events li.on').forEach(function(n){
      n.classList.remove('on');
    });
    var card=document.getElementById('event-'+id);
    var mark=document.querySelector('.ev[data-event="'+CSS.escape(id)+'"]');
    if(card){card.classList.add('on');card.scrollIntoView({block:'center'});}
    if(mark)mark.classList.add('on');
  }
  document.querySelectorAll('.ev').forEach(function(g){
    g.addEventListener('click',function(){selectEvent(g.getAttribute('data-event'));});
    g.addEventListener('keydown',function(e){
      if(e.key==='Enter'||e.key===' '){e.preventDefault();selectEvent(g.getAttribute('data-event'));}
    });
  });
  document.querySelectorAll('.events li').forEach(function(li){
    li.addEventListener('click',function(){selectEvent(li.id.replace('event-',''));});
  });
})();
(function(){
  var root=document.documentElement;

  /* -- dark only --------------------------------------------------------- */
  /* The script above is frozen -- it is `report.py`'s, character for character,
     and the parity legs compare it -- so it still restores a theme from
     localStorage under a key an older report may well have written. This
     document has no `html.light` block to restore into, and no switch to get
     back from it, so a stored 'light' would leave a page with no colours at
     all. Undo it, and forget the key so it cannot happen twice. */
  root.classList.remove('light');
  root.classList.add('dark');
  try{localStorage.removeItem('tg-report-theme');}catch(e){}

  /* -- presence tooltips ------------------------------------------------- */
  /* The compact rows carry their counts once per row rather than once per
     cell, which is most of the size budget. Every bucket is the same width, so
     the pointer's x IS the bucket index; the label comes from the one shared
     axis list. This also answers over the silent days, where the old per-cell
     tooltip had no element to hover at all. */
  var axisEl=document.getElementById('axis');
  var DAYS=axisEl?(axisEl.getAttribute('data-days')||'').split('|'):[];
  var tip=document.getElementById('tip');
  function counts(svg){
    if(svg._n)return svg._n;
    var out={},raw=svg.getAttribute('data-n')||'';
    raw.split(',').forEach(function(pair){
      if(!pair)return;
      var bits=pair.split(':');
      out[bits[0]]=bits[1];
    });
    svg._n=out;return out;
  }
  document.addEventListener('mousemove',function(e){
    var svg=e.target.closest?e.target.closest('svg.strip'):null;
    if(!svg||!svg.hasAttribute('data-n'))return;
    var box=svg.getBoundingClientRect();
    var total=parseInt(svg.getAttribute('data-b'),10)||1;
    var i=Math.floor((e.clientX-box.left)/box.width*total);
    if(i<0)i=0; if(i>=total)i=total-1;
    var n=counts(svg)[i]||'0';
    var label=DAYS[i]||'';
    tip.textContent=label?label+' · '+n:n;
    tip.classList.add('on');
    tip.style.left=e.clientX+'px';tip.style.top=e.clientY+'px';
  },true);

  /* -- kind filters ------------------------------------------------------ */
  document.querySelectorAll('.kind-filter').forEach(function(b){
    b.addEventListener('click',function(){
      var on=b.getAttribute('aria-pressed')!=='true';
      b.setAttribute('aria-pressed',on?'true':'false');
      root.classList.toggle('hide-k'+b.getAttribute('data-k'),!on);
    });
  });

  /* -- search ------------------------------------------------------------ */
  /* Over the text already on the page, so it cannot disagree with what is
     rendered. Rows are collected once: re-querying the DOM on every keystroke
     is what makes a 250-row table feel broken while you type. */
  var find=document.getElementById('find');
  var found=document.getElementById('found');
  if(find){
    var groups=[];
    document.querySelectorAll('#people tbody tr,#topics tbody tr').forEach(function(n){
      groups.push(n);
    });
    document.querySelectorAll('.events li,.records li').forEach(function(n){
      groups.push(n);
    });
    var hay=groups.map(function(n){return (n.textContent||'').toLowerCase();});
    function run(){
      var q=find.value.trim().toLowerCase();
      root.classList.toggle('searching',q.length>0);
      if(!q){
        groups.forEach(function(n){n.classList.remove('nomatch');});
        found.textContent='';
        return;
      }
      var hits=0;
      for(var i=0;i<groups.length;i++){
        var ok=hay[i].indexOf(q)!==-1;
        groups[i].classList.toggle('nomatch',!ok);
        if(ok)hits++;
      }
      found.textContent=hits+' of '+groups.length;
    }
    find.addEventListener('input',run);
    find.addEventListener('keydown',function(e){
      if(e.key==='Escape'){find.value='';run();}
    });
  }
})();
