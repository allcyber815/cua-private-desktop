using System;
using System.Drawing;
using System.Windows.Forms;

public sealed class BgFixture : Form {
    public TextBox Input;
    public Button ActionButton;
    public Label Status;
    public BgFixture() {
        Text = "WebGPT Background Fixture";
        Width = 520; Height = 220;
        StartPosition = FormStartPosition.CenterScreen;
        Input = new TextBox { Name = "FixtureInput", Left = 20, Top = 20, Width = 450, Text = "" };
        ActionButton = new Button { Name = "FixtureButton", Left = 20, Top = 60, Width = 160, Text = "Background Action" };
        Status = new Label { Name = "FixtureStatus", Left = 20, Top = 105, Width = 450, Text = "idle" };
        ActionButton.Click += (s,e) => Status.Text = "clicked";
        Controls.Add(Input); Controls.Add(ActionButton); Controls.Add(Status);
    }
    [STAThread] public static void Main() {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new BgFixture());
    }
}