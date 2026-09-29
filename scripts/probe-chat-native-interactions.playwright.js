async page => {
 const results=[];
 for(let i=0;i<3;i++) {
  const state=await page.evaluate(()=>window.realRunProbe.report().current);
  const collapsed=page.getByRole('button',{name:'展开侧栏',exact:true});
  if(await collapsed.count()) await collapsed.click();
  await page.getByRole('button',{name:'收起侧栏',exact:true}).click();
  await page.getByRole('button',{name:'展开侧栏',exact:true}).waitFor();
  await page.getByRole('button',{name:'展开侧栏',exact:true}).click();
  await page.getByRole('button',{name:'收起侧栏',exact:true}).waitFor();
  const closePanel=page.getByRole('button',{name:'关闭面板',exact:true});
  if(await closePanel.count()) await closePanel.click();
  await page.getByRole('button',{name:'面板',exact:true}).click();
  await closePanel.waitFor();
  await page.getByRole('button',{name:'文件',exact:true}).click();
  await page.waitForTimeout(250);
  const previewClose=page.getByRole('complementary').last().getByRole('button',{name:'关闭',exact:true});
  if(await previewClose.count()) await previewClose.click();
  const group=page.getByRole('button',{name:'group-a',exact:true});
  if(!await group.count()) await page.getByRole('button',{name:'samples',exact:true}).click();
  const file=page.getByRole('button',{name:'case-002.md',exact:true});
  if(await group.locator('svg.lucide-chevron-down').count()) {await group.click();await file.waitFor({state:'hidden'});}
  await group.click();await file.waitFor();
  await file.click();await page.getByText('# Case 2',{exact:true}).waitFor();
  const panel=page.getByRole('complementary').last();
  await panel.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByText('# Case 2',{exact:true}).waitFor({state:'hidden'});
  const viewport=page.locator('.chat-scroll-viewport');
  await viewport.hover();
  const before=await viewport.evaluate(e=>({top:e.scrollTop,height:e.scrollHeight,viewport:e.clientHeight}));
  await page.mouse.wheel(0,-500);
  await page.waitForTimeout(200);
  const after=await viewport.evaluate(e=>e.scrollTop);
  if(before.top<=1 || after>=before.top-1) throw new Error('Long-history viewport did not scroll upward');
  await page.mouse.wheel(0,600);
  results.push({round:i,...state,scroll:{before,after,moved:before.top!==after}});
  await page.waitForTimeout(1000);
 }
 await page.evaluate(results=>{window.realInteractions=[...(window.realInteractions||[]),...results]},results);
 return results;
}



