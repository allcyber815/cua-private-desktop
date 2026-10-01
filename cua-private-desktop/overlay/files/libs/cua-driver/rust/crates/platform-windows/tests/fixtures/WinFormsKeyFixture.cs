using System;
using System.Windows.Forms;

public sealed class WinFormsKeyFixture : Form {
    private readonly Label Status;
    private bool CtrlDown;
    private int LeftDownCount;
    private bool Dragging;
    private bool DragMoved;

    public WinFormsKeyFixture() {
        Text = "WebGPT Background Fixture WinForms Key";
        Width = 560;
        Height = 220;
        KeyPreview = true;

        Status = new Label {
            Name = "FixtureKeyStatus",
            Text = "idle",
            AutoSize = true,
            Left = 24,
            Top = 48
        };

        var menu = new MenuStrip {
            Name = "FixtureMenuStrip"
        };
        var actions = new ToolStripMenuItem("Actions") {
            Name = "FixtureActionsMenu"
        };
        var mark = new ToolStripMenuItem("Mark") {
            Name = "FixtureMarkMenuItem"
        };
        mark.Click += delegate { Status.Text = "menu=mark"; };
        actions.DropDownItems.Add(mark);
        menu.Items.Add(actions);
        MainMenuStrip = menu;
        Controls.Add(menu);
        Controls.Add(Status);
    }

    protected override void WndProc(ref Message m) {
        const int WM_KEYDOWN = 0x0100;
        const int WM_KEYUP = 0x0101;
        const int WM_SYSKEYDOWN = 0x0104;
        const int WM_SYSKEYUP = 0x0105;
        const int WM_HSCROLL = 0x0114;
        const int WM_VSCROLL = 0x0115;
        const int WM_MOUSEMOVE = 0x0200;
        const int WM_LBUTTONDOWN = 0x0201;
        const int WM_LBUTTONUP = 0x0202;
        const int WM_LBUTTONDBLCLK = 0x0203;
        const int WM_RBUTTONDOWN = 0x0204;
        if (m.Msg == WM_KEYDOWN || m.Msg == WM_SYSKEYDOWN) {
            Keys key = (Keys)m.WParam.ToInt32();
            if (key == Keys.ControlKey) {
                CtrlDown = true;
                Status.Text = "key=ControlKey";
            } else if (CtrlDown) {
                Status.Text = "combo=Ctrl+" + key.ToString();
            } else {
                Status.Text = "key=" + key.ToString();
            }
        } else if (m.Msg == WM_KEYUP || m.Msg == WM_SYSKEYUP) {
            if ((Keys)m.WParam.ToInt32() == Keys.ControlKey) {
                CtrlDown = false;
            }
        } else if (m.Msg == WM_VSCROLL || m.Msg == WM_HSCROLL) {
            int code = m.WParam.ToInt32() & 0xffff;
            Status.Text = "scroll=" + (m.Msg == WM_VSCROLL ? "v" : "h") + "-" + code;
        } else if (m.Msg == WM_LBUTTONDBLCLK) {
            Status.Text = "mouse=double-left";
        } else if (m.Msg == WM_LBUTTONDOWN) {
            LeftDownCount++;
            Dragging = true;
            DragMoved = false;
            Status.Text = "mouse=left-" + LeftDownCount.ToString();
        } else if (m.Msg == WM_MOUSEMOVE && Dragging && (m.WParam.ToInt64() & 0x0001) != 0) {
            DragMoved = true;
            Status.Text = "mouse=drag";
        } else if (m.Msg == WM_MOUSEMOVE) {
            Status.Text = "mouse=move";
        } else if (m.Msg == WM_LBUTTONUP) {
            if (Dragging && DragMoved) {
                Status.Text = "mouse=drag-end";
            }
            Dragging = false;
            DragMoved = false;
        } else if (m.Msg == WM_RBUTTONDOWN) {
            Status.Text = "mouse=right";
        }
        base.WndProc(ref m);
    }

    [STAThread]
    public static void Main() {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new WinFormsKeyFixture());
    }
}
