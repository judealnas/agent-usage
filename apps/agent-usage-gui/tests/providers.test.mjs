import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";

const bundle = await build({
  entryPoints: [fileURLToPath(new URL("../src/main.ts", import.meta.url))],
  bundle: true, write: false, format: "iife", plugins: [{
    name: "tauri-test-host",
    setup(builder) {
      builder.onResolve({filter: /^@tauri-apps\/api\//}, args => ({path: args.path, namespace:"mock"}));
      builder.onLoad({filter: /.*/, namespace:"mock"}, () => ({contents: `
        export const invoke=(command,args)=>window.host.invoke(command,args);
        export const listen=async(event,callback)=>{window.host.listener=callback;return ()=>{};};
        export const getCurrentWindow=()=>window.host.appWindow;
        export class LogicalSize { constructor(width,height){this.width=width;this.height=height;} }
        export class PhysicalSize extends LogicalSize {}
      `}));
    },
  }],
});
const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
const settle = () => new Promise(resolve => setImmediate(resolve));
async function setup(cli = {supported:true,available:true,enabled:false,managed:false,conflicts:[]}) {
  const dom = new JSDOM(html, {runScripts:"outside-only",url:"http://localhost"});
  const win = dom.window;
  const calls=[];
  const resizeRequests=[];
  let nativeSize={width:425,height:325};
  const appWindow={
    innerSize:async()=>nativeSize,
    setSize:async(size)=>{
      resizeRequests.push(size);
      if (nativeSize.width===size.width && nativeSize.height===size.height) return;
      nativeSize={width:size.width,height:size.height};
      win.dispatchEvent(new win.Event("resize"));
    },
    minimize:async()=>{},
    close:async()=>{},
  };
  let settings={providers:[{id:"entry-1",name:null,config:{kind:"codex_inference",options:{model:"test-model",effort:"low",auth_file:null}},generation:0,shown:true,auto_refresh:true,interval_seconds:300,tray_enabled:false,tray_group:null}],thresholds:{critical:90},warn_fast_polling:true,pinned:true,close_to_tray:true};
  win.ResizeObserver=class {observe(){}};
  win.host={listener:null,appWindow,invoke:async(command,args)=>{
    calls.push({command,args});
    if(command==="get_meta")return {providers:[{id:"codex",name:"Codex"},{id:"claude",name:"Claude"}],codex_efforts:["low","high"]};
    if(command==="get_cli_status") return structuredClone(cli);
    if(command==="set_cli_enabled"){ if(cli.conflicts.length && args.enabled) throw new Error("Independent CLI found"); cli.enabled=args.enabled; cli.managed=args.enabled; return structuredClone(cli); }
    if(command==="get_settings")return structuredClone(settings);
    if(command==="save_entry"){
      const draft=args.entry;
      const existing=settings.providers.find(p=>p.id===draft.id);
      if(existing){if(JSON.stringify(existing.config)!==JSON.stringify(draft.config))existing.generation++;existing.config=draft.config;existing.name=draft.name;}
      else settings.providers.push({...draft,id:"entry-2",generation:0,shown:true,auto_refresh:true,interval_seconds:300});
      const saved=settings.providers.find(p=>p.id===(draft.id ?? "entry-2"));
      if(args.tray) Object.assign(saved,{tray_enabled:args.tray.enabled,tray_group:args.tray.group});
      return structuredClone(settings);
    }
    if(command==="remove_entry"){settings.providers=settings.providers.filter(p=>p.id!==args.id);return structuredClone(settings);}
    if(command==="save_preferences"){Object.assign(settings,args.preferences);return structuredClone(settings);}

  }};
  win.eval(bundle.outputFiles[0].text);
  await settle();
  const doc=win.document;
  const button=name=>[...doc.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")===name || b.textContent===name);
  const input=label=>[...doc.querySelectorAll("label")].find(l=>l.firstElementChild?.textContent===label)?.querySelector("input,select");
  const change=(field,value)=>{field.value=value;field.dispatchEvent(new win.Event("change",{bubbles:true}));};
  const submit=async()=>{doc.querySelector("form").dispatchEvent(new win.Event("submit",{bubbles:true,cancelable:true}));await settle();};
  return {dom,win,doc,calls,button,input,change,submit,resizeRequests};
}

test("fitting the height preserves the native width on a scaled display", async()=>{
  const t=await setup();
  try {
    Object.defineProperties(t.win, {
      devicePixelRatio:{value:1.25},
      innerWidth:{value:339},
      innerHeight:{value:260},
    });
    t.doc.getElementById("app").getBoundingClientRect=()=>({height:100.2});
    t.win.dispatchEvent(new t.win.Event("resize"));
    await settle();
    assert.equal(t.resizeRequests.length,1);
    assert.equal(t.resizeRequests[0].width,425);
    assert.equal(t.resizeRequests[0].height,126);
  } finally {
    t.win.close();
  }
});

test("provider editor shows only supported types and strategy-specific settings",async()=>{
  const t=await setup();try{
    t.button("Settings").click();t.button("Add provider").click();
    assert.equal(t.input("Name (optional)").value,"");
    assert.deepEqual([...t.input("Provider").options].map(o=>o.value),["codex","claude"]);
    assert.ok(t.input("Model"));assert.equal(t.input("Codex executable"),undefined);
    t.change(t.input("Strategy"),"account");
    assert.ok(t.input("Codex executable"));assert.equal(t.input("Model"),undefined);
    assert.equal(t.doc.querySelector(".interval"),null);
    t.input("Name (optional)").value="My account";
    await t.submit();
    const saved=t.calls.find(c=>c.command==="save_entry").args.entry;
    assert.equal(saved.config.kind,"codex_account");assert.equal(saved.name,"My account");
    assert.equal(saved.config.options.model,undefined);
    assert.ok(t.doc.querySelector(".provider-entry").textContent.includes("Codex"));
  }finally{t.dom.window.close();}
});

test("cancel does not save; renaming does not refresh; remove clears the entry",async()=>{
  const t=await setup();try{
    t.button("Settings").click();t.doc.querySelector(".provider-entry").click();
    t.input("Name (optional)").value="Discard";t.button("Cancel").click();
    assert.equal(t.calls.filter(c=>c.command==="save_entry").length,0);
    t.doc.querySelector(".provider-entry").click();t.input("Name (optional)").value="Work";await t.submit();
    assert.equal(t.calls.filter(c=>c.command==="refresh").length,0);
    t.doc.querySelector(".provider-entry").click();t.button("Remove").click();await settle();
    assert.equal(t.doc.querySelectorAll(".provider-entry").length,0);
  }finally{t.dom.window.close();}
});

test("obsolete reports are ignored and elapsed reset times retain measured usage",async()=>{
  const t=await setup();try{
    const report={provider:"codex",plan:null,metrics:[{kind:"window",label:"Test",used_percent:42,remaining_percent:58,duration_minutes:300,resets_at:1}],limit_reached:false,details:{},fetched_at:0,error:null};
    t.win.host.listener({payload:{entry_id:"entry-1",generation:99,report,source_report:null}});
    assert.ok(!t.doc.querySelector(".card-body").textContent.includes("42%"));
    t.win.host.listener({payload:{entry_id:"entry-1",generation:0,report,source_report:null}});
    assert.ok(t.doc.querySelector(".card-body").textContent.includes("42%"));
    assert.ok(t.doc.querySelector(".card-body").textContent.includes("awaiting refresh"));
    t.win.host.listener({payload:{entry_id:"entry-1",generation:0,report:{...report,error:"Offline"},source_report:null}});
    assert.ok(t.doc.querySelector(".card").classList.contains("stale"));
    assert.ok(t.doc.querySelector(".card-body").textContent.includes("42%"));
  }finally{t.dom.window.close();}
});

test("CLI conflict disables enabling and Check again rechecks without changing installations", async()=>{
  const cli={supported:true,available:true,enabled:false,managed:false,conflicts:["C:\\Tools\\agent-usage-cli.exe"]};
  const t=await setup(cli);try{
    t.button("Settings").click();await settle();
    const toggle=t.doc.querySelector('[aria-label="Enable command line interface"]');
    assert.equal(toggle.disabled,true);
    assert.ok(t.doc.querySelector(".cli-conflicts").textContent.includes(cli.conflicts[0]));
    assert.equal(t.calls.filter(c=>c.command==="set_cli_enabled").length,0);
    cli.conflicts=[];t.button("Check again").click();await settle();
    assert.equal(toggle.disabled,false);
    toggle.click();await settle();
    assert.equal(toggle.checked,true);
    assert.deepEqual(t.calls.filter(c=>c.command==="set_cli_enabled").map(c=>c.args.enabled),[true]);
    cli.conflicts=["C:\\Other\\agent-usage-cli.exe"];
    t.button("Check again").click();await settle();
    assert.equal(toggle.disabled,false); // still allow removing our own registration
    toggle.click();await settle();
    assert.equal(toggle.checked,false);assert.equal(toggle.disabled,true);
  }finally{t.dom.window.close();}
});

test("CLI enable failure refreshes the conflict warning and restores the checkbox", async()=>{
  const cli={supported:true,available:true,enabled:false,managed:false,conflicts:[]};
  const t=await setup(cli);try{
    t.button("Settings").click();await settle();
    const toggle=t.doc.querySelector('[aria-label="Enable command line interface"]');
    cli.conflicts=["C:\\Tools\\agent-usage-cli.exe"];
    toggle.click();await settle();
    assert.equal(toggle.checked,false);assert.equal(toggle.disabled,true);
    assert.match(t.doc.querySelector(".cli-settings .form-error").textContent,/Independent CLI/);
  }finally{t.dom.window.close();}
});


test("settings headings sit outside cards and General changes save without leaving settings", async()=>{
  const t=await setup();try{
    t.button("Settings").click();await settle();
    const groups=[...t.doc.querySelectorAll(".settings-group")];
    assert.deepEqual(groups.map(g=>g.firstElementChild.textContent),["General","Providers","About"]);
    assert.ok(groups.every(g=>g.lastElementChild.classList.contains("settings-section")));
    assert.equal(t.button("Save"),undefined);assert.equal(t.button("Cancel"),undefined);
    t.change(t.input("Red at (% used)"),"75");await settle();
    const saves=t.calls.filter(c=>c.command==="save_preferences");
    assert.equal(saves.length,1);assert.equal(saves[0].args.preferences.thresholds.critical,75);
    assert.ok(t.doc.querySelector(".settings-overview"));
    const warning=t.doc.querySelector(".general-settings .settings-option .switch input");
    warning.click();await settle();
    assert.equal(t.calls.filter(c=>c.command==="save_preferences").at(-1).args.preferences.warn_fast_polling,false);
    t.change(t.input("Red at (% used)"),"0");await settle();
    assert.equal(t.calls.filter(c=>c.command==="save_preferences").length,2);
    t.button("Add provider").click();assert.ok(t.button("Save"));assert.ok(t.button("Cancel"));
  }finally{t.dom.window.close();}
});

test("failed automatic save restores the saved value and shows an error", async()=>{
  const t=await setup();try{
    const invoke=t.win.host.invoke;
    t.win.host.invoke=async(command,args)=>{if(command==="save_preferences")throw new Error("Disk unavailable");return invoke(command,args);};
    t.button("Settings").click();await settle();
    t.change(t.input("Red at (% used)"),"70");await settle();
    assert.equal(t.input("Red at (% used)").value,"90");
    assert.equal(t.input("Red at (% used)").disabled,false);
    assert.match(t.doc.querySelector(".general-settings > .form-error").textContent,/Disk unavailable/);
  }finally{t.dom.window.close();}
});

test("tray settings live in provider configuration and respect Save, Cancel, and save failures", async()=>{
  const t=await setup();try{
    t.button("Settings").click();await settle();
    const checkbox=text=>[...t.doc.querySelectorAll("label.switch")].find(l=>l.textContent===text)?.querySelector("input");
    const close=checkbox("Close to system tray");
    assert.equal(close.checked,true);close.click();await settle();
    assert.equal(t.calls.filter(c=>c.command==="save_preferences").at(-1).args.preferences.close_to_tray,false);
    assert.equal(checkbox("Show in system tray"),undefined);
    t.doc.querySelector(".provider-entry").click();
    checkbox("Show in system tray").click();
    assert.equal(t.calls.filter(c=>c.command==="save_entry").length,0);
    t.button("Cancel").click();t.doc.querySelector(".provider-entry").click();
    assert.equal(checkbox("Show in system tray").checked,false);
    checkbox("Show in system tray").click();await t.submit();
    assert.equal(t.calls.filter(c=>c.command==="save_entry").at(-1).args.tray.enabled,true);
    t.doc.querySelector(".provider-entry").click();
    assert.equal(checkbox("Show in system tray").checked,true);
    const original=t.win.host.invoke;
    t.win.host.invoke=(command,args)=>command==="save_entry"?Promise.reject(new Error("Disk unavailable")):original(command,args);
    checkbox("Show in system tray").click();await t.submit();
    assert.match(t.doc.querySelector(".form-error").textContent,/Disk unavailable/);
    t.button("Cancel").click();t.doc.querySelector(".provider-entry").click();
    assert.equal(checkbox("Show in system tray").checked,true);
  }finally{t.dom.window.close();}
});

test("new providers can select tray visibility before their first save", async()=>{
  const t=await setup();try{
    t.button("Settings").click();t.button("Add provider").click();
    const toggle=[...t.doc.querySelectorAll("label.switch")].find(l=>l.textContent==="Show in system tray").querySelector("input");
    toggle.click();await t.submit();
    assert.equal(t.calls.find(c=>c.command==="save_entry").args.tray.enabled,true);
  }finally{t.dom.window.close();}
});
