using System;
using System.IO;
using System.Text;
using System.Windows.Automation;
using System.Runtime.InteropServices;

public static class PrivateDesktopUiaDumpWorker {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd,out uint pid);
    static uint FgPid(){uint p=0;var h=GetForegroundWindow();if(h!=IntPtr.Zero)GetWindowThreadProcessId(h,out p);return p;}

    [STAThread] public static void Main(string[] args){
        string output=args[2];
        try{
            int targetPid=Int32.Parse(args[0]);
            IntPtr hwnd=new IntPtr(Int64.Parse(args[1]));
            var root=AutomationElement.FromHandle(hwnd);
            if(root==null)throw new Exception("FromHandle null");
            var all=root.FindAll(TreeScope.Descendants,Condition.TrueCondition);
            var sb=new StringBuilder();
            sb.Append("ok=true;rootPid=").Append(root.Current.ProcessId)
              .Append(";expectedPid=").Append(targetPid)
              .Append(";descendants=").Append(all.Count)
              .Append(";privateFg=").Append(FgPid());
            int n=Math.Min(all.Count,120);
            for(int i=0;i<n;i++){
                var e=all[i];
                sb.Append("|").Append(i)
                  .Append(":name=").Append(e.Current.Name)
                  .Append(",aid=").Append(e.Current.AutomationId)
                  .Append(",class=").Append(e.Current.ClassName)
                  .Append(",type=").Append(e.Current.ControlType.ProgrammaticName)
                  .Append(",hwnd=").Append(e.Current.NativeWindowHandle);
            }
            File.WriteAllText(output,sb.ToString());
        }catch(Exception ex){
            File.WriteAllText(output,"ok=false;error="+ex.GetType().Name+":"+ex.Message);
        }
    }
}