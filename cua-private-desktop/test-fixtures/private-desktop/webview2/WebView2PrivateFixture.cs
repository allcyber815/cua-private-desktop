using System;
using System.IO;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.WinForms;

public sealed class WebView2PrivateFixture : Form
{
    readonly WebView2 web = new WebView2();

    const string Html = @"<!doctype html>
<html><head><meta charset='utf-8'><title>WebGPT WebView2 Private Fixture</title>
<style>
body{font-family:sans-serif;background:#f0f0f0;margin:0}
main{margin:24px;padding:28px;background:#fff;border:4px solid #222;font-size:20px}
label,button,select{display:block;margin:14px 0}
input,button,select{font-size:20px;padding:7px}
#fixture-status{margin-top:18px;padding:10px;border:2px solid #555;min-height:26px}
</style></head><body><main>
<h1>WebGPT WebView2 Private Fixture</h1>
<label>Input <input id='fixture-input' aria-label='Fixture Input' value=''></label>
<button id='fixture-button'>Background Action</button>
<label><input id='fixture-check' type='checkbox'> Background Check</label>
<label>Fixture Select
<select id='fixture-select'><option>Red</option><option>Green</option><option>Blue</option></select>
</label>
<div id='fixture-status' role='status'>idle</div>
<script>
(()=>{
 const s=document.getElementById('fixture-status');
 const i=document.getElementById('fixture-input');
 const b=document.getElementById('fixture-button');
 const c=document.getElementById('fixture-check');
 const sel=document.getElementById('fixture-select');
 let clicks=0;
 i.addEventListener('input',()=>s.textContent='text='+i.value);
 i.addEventListener('change',()=>s.textContent='text='+i.value);
 b.addEventListener('click',()=>s.textContent='clicked-'+(++clicks));
 c.addEventListener('change',()=>s.textContent='check='+c.checked);
 sel.addEventListener('change',()=>s.textContent='selected='+sel.value);
})();
</script></main></body></html>";

    public WebView2PrivateFixture()
    {
        Text = "WebGPT WebView2 Private Fixture";
        Width = 945;
        Height = 760;
        StartPosition = FormStartPosition.Manual;
        Left = 80;
        Top = 80;
        web.Dock = DockStyle.Fill;
        Controls.Add(web);
        Shown += async delegate { await InitAsync(); };
    }

    async Task InitAsync()
    {
        Environment.SetEnvironmentVariable("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--force-renderer-accessibility");
        string userData = Path.Combine(Path.GetTempPath(), "webgpt-cua-webview2-" + Environment.ProcessId);
        if (Directory.Exists(userData)) Directory.Delete(userData, true);
        Directory.CreateDirectory(userData);
        var env = await CoreWebView2Environment.CreateAsync(null, userData);
        await web.EnsureCoreWebView2Async(env);
        web.CoreWebView2.Settings.AreDefaultContextMenusEnabled = false;
        web.CoreWebView2.Settings.AreDevToolsEnabled = false;
        web.NavigateToString(Html);
    }

    [STAThread]
    public static void Main()
    {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new WebView2PrivateFixture());
    }
}
