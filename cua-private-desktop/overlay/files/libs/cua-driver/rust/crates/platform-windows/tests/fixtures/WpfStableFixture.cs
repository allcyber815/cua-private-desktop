using System;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;

public sealed class WpfStableFixture : Window {
    TextBox Input;
    Button Button;
    Button FunctionButton;
    CheckBox Check;
    ComboBox Combo;
    Label Status;
    int ClickCount;

    public WpfStableFixture() {
        Title = "WebGPT Background Fixture WPF";
        Width = 640;
        Height = 360;
        UseLayoutRounding = true;
        SnapsToDevicePixels = true;

        var panel = new StackPanel { Margin = new Thickness(20) };

        Input = new TextBox {
            Name = "FixtureInput",
            Width = 560,
            Margin = new Thickness(0, 0, 0, 8),
            Focusable = false
        };
        AutomationProperties.SetAutomationId(Input, "FixtureInput");
        AutomationProperties.SetName(Input, "Fixture Input");

        Button = new Button {
            Name = "FixtureButton",
            Content = "Background Action",
            Width = 180,
            Margin = new Thickness(0, 0, 0, 8),
            HorizontalAlignment = HorizontalAlignment.Left,
            Focusable = false
        };
        AutomationProperties.SetAutomationId(Button, "FixtureButton");
        AutomationProperties.SetAcceleratorKey(Button, "Ctrl+K");

        FunctionButton = new Button {
            Name = "FixtureFunctionButton",
            Content = "Function Action",
            Width = 180,
            Margin = new Thickness(0, 0, 0, 8),
            HorizontalAlignment = HorizontalAlignment.Left,
            Focusable = false
        };
        AutomationProperties.SetAutomationId(FunctionButton, "FixtureFunctionButton");
        AutomationProperties.SetAcceleratorKey(FunctionButton, "F6");

        Check = new CheckBox {
            Name = "FixtureCheck",
            Content = "Background Check",
            Margin = new Thickness(0, 0, 0, 8),
            Focusable = false
        };
        AutomationProperties.SetAutomationId(Check, "FixtureCheck");

        Combo = new ComboBox {
            Name = "FixtureCombo",
            Width = 220,
            Margin = new Thickness(0, 0, 0, 8),
            HorizontalAlignment = HorizontalAlignment.Left,
            Focusable = false
        };
        Combo.Items.Add("Red");
        Combo.Items.Add("Green");
        Combo.Items.Add("Blue");
        Combo.SelectedIndex = 0;
        AutomationProperties.SetAutomationId(Combo, "FixtureCombo");

        Status = new Label { Name = "FixtureStatus", Content = "idle" };
        AutomationProperties.SetAutomationId(Status, "FixtureStatus");

        panel.Children.Add(Input);
        panel.Children.Add(Button);
        panel.Children.Add(FunctionButton);
        panel.Children.Add(Check);
        panel.Children.Add(Combo);
        panel.Children.Add(Status);
        Content = panel;

        Input.TextChanged += (s, e) => {
            Status.Content = "text=" + Input.Text;
            Input.InvalidateVisual();
            Status.InvalidateVisual();
            Input.UpdateLayout();
            Status.UpdateLayout();
        };
        Button.Click += (s, e) => {
            ClickCount++;
            Status.Content = "clicked-" + ClickCount;
        };
        FunctionButton.Click += (s, e) => Status.Content = "function-1";
        Check.Checked += (s, e) => Status.Content = "check=true";
        Check.Unchecked += (s, e) => Status.Content = "check=false";
        Combo.SelectionChanged += (s, e) => Status.Content = "selected=" + Combo.SelectedItem;
    }

    [STAThread]
    public static void Main() {
        RenderOptions.ProcessRenderMode = RenderMode.SoftwareOnly;
        var app = new Application();
        app.Run(new WpfStableFixture());
    }
}
