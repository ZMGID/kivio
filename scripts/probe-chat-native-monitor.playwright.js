async page => {
  await page.evaluate(async () => {
    if (window.realRunProbe) throw new Error('Stop the existing realRunProbe before starting another sample');
    const store = await import('/src/chat/streamingStore.ts');
    const started = performance.now();
    const data = {startedAt: new Date().toISOString(), environment:navigator.userAgent, clicks:[], tasks:[], samples:[], updates:0, maxTools:0,maxSegments:0,maxContent:0};
    const sample=()=>{const s=store.getSnapshot();const c=store.getCoarse();data.maxTools=Math.max(data.maxTools,s.toolCalls.length);data.maxSegments=Math.max(data.maxSegments,s.segments.length);data.maxContent=Math.max(data.maxContent,s.content.length);return {atMs:performance.now()-started,streaming:c.streaming,tools:s.toolCalls.length,segments:s.segments.length,content:s.content.length,blocks:document.querySelectorAll('.chat-markdown').length,dom:document.querySelectorAll('*').length,error:c.streamError};};
    const unsubscribe=store.subscribeSnapshot(()=>data.updates++);
    const observer=new PerformanceObserver(list=>{for(const e of list.getEntries())data.tasks.push({atMs:e.startTime-started,duration:e.duration,streaming:store.getCoarse().streaming});});
    observer.observe({entryTypes:['longtask']});
    const onClick=e=>{const b=e.target.closest('button,[role=button]');if(!b)return;const label=b.getAttribute('aria-label')||b.getAttribute('title')||b.innerText.slice(0,70);const t=performance.now();const current=sample();requestAnimationFrame(()=>requestAnimationFrame(()=>data.clicks.push({label,atMs:t-started,streaming:current.streaming,tools:current.tools,segments:current.segments,inputToHandlerMs:t-e.timeStamp,handlerToSecondFrameMs:performance.now()-t})));};
    document.addEventListener('click',onClick,true);
    const timer=setInterval(()=>data.samples.push(sample()),1000);
    window.realRunProbe={report:()=>({...data,current:sample(),url:location.hash}),stop:()=>{clearInterval(timer);observer.disconnect();unsubscribe();document.removeEventListener('click',onClick,true);}};
  });
}

