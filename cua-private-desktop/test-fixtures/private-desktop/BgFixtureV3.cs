using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public sealed class BgFixtureV3 : Form {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);

    public CheckBox Check;
    public ListBox List;
    public Label Status;
    public Label Events;
    int activated=0,deactivated=0,checkChanged=0,selectionChanged=0;
    string last="init";

    public BgFixtureV3() {
        Text="WebGPT Background Fixture V3";
        Width=640; Height=340; StartPosition=FormStartPosition.CenterScreen;
        Check=new CheckBox { Name="FixtureCheck", Left=20, Top=20, Width=220, Text="Background Check" };
        List=new ListBox { Name="FixtureList", Left=20, Top=60, Width=220, Height=100 };
        List.Items.AddRange(new object[]{"Alpha","Beta","Gamma"});
        Status=new Label { Name="FixtureStatus", Left=280, Top=20, Width=320, Height=60, Text="idle" };
        Events=new Label { Name="FixtureEvents", Left=20, Top=190, Width=580, Height=80, Text="" };

        Activated += (s,e)=>{activated++;last="form-activated";RefreshEvents();};
        Deactivate += (s,e)=>{deactivated++;last="form-deactivated";RefreshEvents();};
        Check.CheckedChanged += (s,e)=>{checkChanged++;Status.Text="check="+Check.Checked;last="check-changed";RefreshEvents();};
        List.SelectedIndexChanged += (s,e)=>{selectionChanged++;Status.Text="selected="+(List.SelectedItem==null?"null":List.SelectedItem.ToString());last="selection-changed";RefreshEvents();};

        Controls.Add(Check); Controls.Add(List); Controls.Add(Status); Controls.Add(Events);
        var timer=new Timer { Interval=100 };
        timer.Tick += (s,e)=>RefreshEvents();
        timer.Start();
        Shown += (s,e)=>RefreshEvents();
    }

    void RefreshEvents() {
        uint fgpid=0; var fg=GetForegroundWindow();
        if(fg!=IntPtr.Zero) GetWindowThreadProcessId(fg,out fgpid);
        Events.Text="activated="+activated+";deactivated="+deactivated+
            ";checkChanged="+checkChanged+";selectionChanged="+selectionChanged+
            ";last="+last+";containsFocus="+ContainsFocus+";fgpid="+fgpid;
    }

    [STAThread] public static void Main() {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new BgFixtureV3());
    }
}