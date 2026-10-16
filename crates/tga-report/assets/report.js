
(function(){
  var root=document.documentElement;
  root.classList.add('js');

  /* -- dark only --------------------------------------------------------- */
  /* An older report kept a theme in localStorage under this key, and a
     stored 'light' would leave this page, which has no light block, with no
     colours at all. Undo it, and forget the key so it cannot happen twice. */
  root.classList.remove('light');
  root.classList.add('dark');
  try{localStorage.removeItem('tg-report-theme');}catch(e){}

  function store(key,value){try{localStorage.setItem(key,value);}catch(e){}}
  function recall(key){try{return localStorage.getItem(key);}catch(e){return null;}}
  var narrow=window.matchMedia?window.matchMedia('(max-width:820px)'):{matches:false};

  /* -- tooltips ---------------------------------------------------------- */
  /* The compact presence rows carry their counts once per row rather than
     once per cell, which is most of the size budget. Every bucket is the same
     width, so the pointer's x IS the bucket index; the label comes from the
     one shared axis list. */
  var tip=document.getElementById('tip');
  var axisEl=document.getElementById('axis');
  var DAYS=axisEl?(axisEl.getAttribute('data-days')||'').split('|'):[];
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
  function tipText(target,x){
    var svg=target.closest?target.closest('svg.strip'):null;
    if(svg&&svg.hasAttribute('data-n')){
      var box=svg.getBoundingClientRect();
      var total=parseInt(svg.getAttribute('data-b'),10)||1;
      var i=Math.floor((x-box.left)/box.width*total);
      if(i<0)i=0;if(i>=total)i=total-1;
      var n=counts(svg)[i]||'0',label=DAYS[i]||'';
      return label?label+' · '+n:n;
    }
    var el=target.closest?target.closest('[data-tip]'):null;
    return el?el.getAttribute('data-tip'):'';
  }
  function show(text,x,y){
    if(!text){tip.classList.remove('on');return;}
    tip.textContent=text;tip.classList.add('on');
    var half=tip.offsetWidth/2+6;
    x=Math.max(half,Math.min(window.innerWidth-half,x));
    tip.style.left=x+'px';tip.style.top=y+'px';
  }
  document.addEventListener('mousemove',function(e){
    show(tipText(e.target,e.clientX),e.clientX,e.clientY);
  });
  /* A finger has no hover, so a tap shows the tip and it stays until the
     next tap or a few seconds pass. */
  var tipTimer=0;
  document.addEventListener('pointerdown',function(e){
    if(e.pointerType==='mouse')return;
    var text=tipText(e.target,e.clientX);
    clearTimeout(tipTimer);
    show(text,e.clientX,e.clientY-12);
    if(text)tipTimer=setTimeout(function(){tip.classList.remove('on');},2600);
  });
  document.addEventListener('scroll',function(){tip.classList.remove('on');},{passive:true});
  document.addEventListener('focusin',function(e){
    var el=e.target.closest?e.target.closest('[data-tip]'):null;
    if(!el){tip.classList.remove('on');return;}
    var r=el.getBoundingClientRect();show(el.getAttribute('data-tip'),r.left+r.width/2,r.top);
  });

  /* -- folding ----------------------------------------------------------- */
  /* Remembered per report, by its title, so folding a section in one
     archive's report does not fold it in every other. */
  var foldKey='tg-report-fold:'+document.title;
  var folded={};
  try{(JSON.parse(recall(foldKey)||'[]')||[]).forEach(function(id){folded[id]=1;});}catch(e){}
  document.querySelectorAll('section[id]').forEach(function(sec){
    var b=sec.querySelector('.fold');
    if(!b)return;
    function set(on){
      sec.classList.toggle('folded',on);
      b.setAttribute('aria-expanded',on?'false':'true');
      b.setAttribute('aria-label',on?'Open this section':'Fold this section');
    }
    if(folded[sec.id])set(true);
    b.addEventListener('click',function(){
      var on=!sec.classList.contains('folded');
      set(on);
      if(on)folded[sec.id]=1;else delete folded[sec.id];
      store(foldKey,JSON.stringify(Object.keys(folded)));
    });
  });

  /* -- the bar ----------------------------------------------------------- */
  var toc=document.querySelector('.bar .toc');
  var links={};
  if(toc)toc.querySelectorAll('a').forEach(function(a){links[a.getAttribute('href').slice(1)]=a;});
  function current(id){
    Object.keys(links).forEach(function(k){links[k].classList.toggle('cur',k===id);});
    var a=links[id];
    if(a&&toc.scrollWidth>toc.clientWidth){
      toc.scrollTo({left:a.offsetLeft-toc.clientWidth/2+a.offsetWidth/2,behavior:'smooth'});
    }
  }
  /* A section folded open by a link keeps its place in the page. */
  if(toc)toc.addEventListener('click',function(e){
    var a=e.target.closest('a');
    if(!a)return;
    var sec=document.getElementById(a.getAttribute('href').slice(1));
    if(sec&&sec.classList.contains('folded')){var b=sec.querySelector('.fold');if(b)b.click();}
  });
  if('IntersectionObserver' in window){
    var seen={};
    var io=new IntersectionObserver(function(entries){
      entries.forEach(function(en){seen[en.target.id]=en.isIntersecting;});
      var ids=Object.keys(links);
      for(var i=0;i<ids.length;i++){if(seen[ids[i]]){current(ids[i]);return;}}
    },{rootMargin:'-56px 0px -60% 0px'});
    Object.keys(links).forEach(function(id){var s=document.getElementById(id);if(s)io.observe(s);});
  }

  /* -- wide charts start at the newest end ------------------------------- */
  document.querySelectorAll('.scrollx.end').forEach(function(f){f.scrollLeft=f.scrollWidth;});

  /* -- events ------------------------------------------------------------ */
  function selectEvent(id){
    document.querySelectorAll('.ev.on,.events li.on').forEach(function(n){n.classList.remove('on');});
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
  document.querySelectorAll('.kind-filter').forEach(function(b){
    b.addEventListener('click',function(){
      var on=b.getAttribute('aria-pressed')!=='true';
      b.setAttribute('aria-pressed',on?'true':'false');
      root.classList.toggle('hide-k'+b.getAttribute('data-k'),!on);
    });
  });

  /* -- former names ------------------------------------------------------ */
  var aliases=document.getElementById('aliases');
  if(aliases)aliases.addEventListener('click',function(){
    var on=root.classList.toggle('aliases');
    aliases.setAttribute('aria-pressed',on?'true':'false');
  });

  /* -- people: filter, show all, search ---------------------------------- */
  /* One function decides which rows show, so the chips, the search box, the
     sort and Show all cannot disagree about it. */
  var SHOWN=30;
  var ptable=document.querySelector('.people-table');
  var prows=ptable?Array.prototype.slice.call(ptable.tBodies[0].rows):[];
  var phay=prows.map(function(r){return (r.textContent||'').toLowerCase();});
  var filter='all',all=false,query='';
  var more=document.getElementById('people-more');
  function applyPeople(){
    if(!ptable)return 0;
    var rows=ptable.tBodies[0].rows,shown=0,hits=0;
    for(var i=0;i<rows.length;i++){
      var r=rows[i],at=prows.indexOf(r);
      var ok=(filter==='all'||r.classList.contains(filter))&&(!query||phay[at].indexOf(query)!==-1);
      r.classList.toggle('off',!ok);
      if(ok){hits++;r.classList.toggle('overflow',!all&&!query&&shown>=SHOWN);shown++;}
      else r.classList.remove('overflow');
    }
    if(more){
      more.hidden=all||!!query||shown<=SHOWN;
      more.textContent='Show all '+shown.toLocaleString('en').replace(/,/g,' ');
    }
    return hits;
  }
  document.querySelectorAll('.ptools .chip[data-f]').forEach(function(c){
    c.addEventListener('click',function(){
      filter=c.getAttribute('data-f');
      document.querySelectorAll('.ptools .chip[data-f]').forEach(function(o){
        o.setAttribute('aria-pressed',o===c?'true':'false');
      });
      applyPeople();
    });
  });
  if(more)more.addEventListener('click',function(){all=true;applyPeople();});

  var find=document.getElementById('find');
  var found=document.getElementById('found');
  if(find){
    var groups=[];
    document.querySelectorAll('#topics tbody tr,.events li,.records li').forEach(function(n){groups.push(n);});
    var hay=groups.map(function(n){return (n.textContent||'').toLowerCase();});
    var run=function(){
      query=find.value.trim().toLowerCase();
      root.classList.toggle('searching',query.length>0);
      var hits=applyPeople();
      if(!query){
        groups.forEach(function(n){n.classList.remove('nomatch');});
        found.textContent='';
        return;
      }
      for(var i=0;i<groups.length;i++){
        var ok=hay[i].indexOf(query)!==-1;
        groups[i].classList.toggle('nomatch',!ok);
        if(ok)hits++;
      }
      found.textContent=hits+' found';
    };
    find.addEventListener('input',run);
    find.addEventListener('keydown',function(e){
      if(e.key==='Escape'){find.value='';run();}
      if(e.key==='Enter'){
        var first=document.querySelector('.people-table tbody tr:not(.off):not(.overflow),#topics tbody tr:not(.nomatch),.events li:not(.nomatch),.records li:not(.nomatch)');
        if(first)first.scrollIntoView({block:'center'});
      }
    });
  }

  /* -- sorting ----------------------------------------------------------- */
  function cellValue(cell,numeric){
    var v=cell.getAttribute('data-v');
    if(v===null)v=cell.textContent||'';
    if(!numeric)return v.toLowerCase();
    var n=parseFloat(v.replace(/[^0-9.\-]/g,''));
    return isNaN(n)?-Infinity:n;
  }
  document.querySelectorAll('th[data-sort]').forEach(function(th){
    th.addEventListener('click',function(){
      var table=th.closest('table'),body=table.tBodies[0];
      var col=Array.prototype.indexOf.call(th.parentNode.children,th);
      var numeric=th.getAttribute('data-sort')==='n';
      /* Numbers start biggest first, words start at A. */
      var desc=th.classList.contains('desc')?false:th.classList.contains('asc')?true:numeric;
      table.querySelectorAll('th.asc,th.desc').forEach(function(o){o.classList.remove('asc','desc');});
      th.classList.add(desc?'desc':'asc');
      var rows=Array.prototype.slice.call(body.rows);
      var keyed=rows.map(function(r,i){return [cellValue(r.cells[col]||r,numeric),i,r];});
      keyed.sort(function(a,b){
        if(a[0]<b[0])return desc?1:-1;
        if(a[0]>b[0])return desc?-1:1;
        return a[1]-b[1];
      });
      keyed.forEach(function(k){body.appendChild(k[2]);});
      if(table===ptable)applyPeople();
    });
  });

  /* -- focus ------------------------------------------------------------- */
  /* One person drawn over the charts: their bars over the timeline, their
     hours over the hour chart, and every row, bar and dot that is theirs
     marked. Only the loudest have bars to draw; anybody can be marked. */
  var data={};
  var dataEl=document.getElementById('focus-data');
  if(dataEl){try{data=JSON.parse(dataEl.textContent)||{};}catch(e){}}
  var pill=document.getElementById('focus');
  var focused='';
  function clearFocus(){
    focused='';
    root.classList.remove('focusing');
    document.querySelectorAll('.hit').forEach(function(n){n.classList.remove('hit');});
    document.querySelectorAll('.fx').forEach(function(n){n.parentNode.removeChild(n);});
    document.querySelectorAll('.has-fx').forEach(function(n){n.classList.remove('has-fx');});
    if(pill)pill.hidden=true;
  }
  function nameOf(key){
    if(data[key])return data[key].name;
    var row=document.querySelector('tr[data-who="'+CSS.escape(key)+'"] .nm');
    return row?row.textContent:key;
  }
  function focus(key){
    if(!key||key===focused){clearFocus();return;}
    clearFocus();
    focused=key;
    root.classList.add('focusing');
    document.querySelectorAll('[data-who]').forEach(function(n){
      if((' '+n.getAttribute('data-who')+' ').indexOf(' '+key+' ')!==-1)n.classList.add('hit');
    });
    var who=data[key];
    var hero=document.querySelector('svg.hero');
    if(who&&hero){
      var b=parseInt(hero.getAttribute('data-b'),10)||1;
      var top=parseFloat(hero.getAttribute('data-top'))||1;
      var h=parseFloat(hero.getAttribute('data-h'))||1;
      var w=hero.viewBox.baseVal.width||1120;
      var slot=w/b,gap=slot>5?2:0,bar=Math.max(1,Math.min(24,slot-gap));
      who.bars.split(',').forEach(function(pair){
        if(!pair)return;
        var bits=pair.split(':'),i=+bits[0],n=+bits[1];
        var tall=Math.max(1.5,h*Math.min(1,n/top));
        var r=document.createElementNS(hero.namespaceURI,'rect');
        r.setAttribute('class','fx');
        r.setAttribute('x',i*slot+(slot-bar)/2);r.setAttribute('y',h-tall);
        r.setAttribute('width',bar);r.setAttribute('height',tall);
        r.setAttribute('rx',Math.min(2,bar/2));
        hero.appendChild(r);
      });
    }
    var hours=document.getElementById('hours-chart');
    if(who&&hours){
      var max=parseFloat(hours.getAttribute('data-max'))||1;
      hours.classList.add('has-fx');
      hours.querySelectorAll('.vc').forEach(function(vc,i){
        var n=who.hours[i]||0;
        if(!n)return;
        var fx=document.createElement('i');
        fx.className='fx';fx.style.height=Math.max(0.8,n/max*100)+'%';
        vc.appendChild(fx);
      });
    }
    if(pill){
      pill.querySelector('.fname').textContent=nameOf(key)+(who?'':' (marked only)');
      pill.hidden=false;
    }
  }
  if(pill)pill.querySelector('button').addEventListener('click',clearFocus);
  document.addEventListener('keydown',function(e){
    if(e.key==='Escape'&&focused&&e.target!==find)clearFocus();
  });
  document.addEventListener('click',function(e){
    var t=e.target;
    if(!t.closest)return;
    /* A person's row on a phone opens first; a tap on the name of an open
       row focuses them. */
    var row=t.closest('table.rows tbody tr');
    if(row&&narrow.matches){
      if(row.classList.contains('open')&&t.closest('.nm')){focus(row.getAttribute('data-who'));return;}
      row.classList.toggle('open');
      return;
    }
    var name=t.closest('tr[data-who] .nm');
    if(name){focus(name.closest('tr').getAttribute('data-who'));return;}
    var one=t.closest('.hbars li[data-who],.node[data-who],.figures li[data-who]');
    if(one){
      var key=one.getAttribute('data-who');
      if(key&&key.indexOf(' ')===-1)focus(key);
    }
  });
})();
