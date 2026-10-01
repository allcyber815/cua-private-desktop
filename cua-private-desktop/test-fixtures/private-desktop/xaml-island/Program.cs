using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.Windows.Forms;

namespace WebGPT.XamlIslandFixture;

internal static class Program
{
    private static void SetId(DependencyObject value, string id) => AutomationProperties.SetAutomationId(value, id);

    [STAThread]
    private static void Main()
    {
        DispatcherQueueController controller = DispatcherQueueController.CreateOnCurrentThread();
        var xamlApp = new XamlApp();
        WindowsAppSdkHelper.EnableContentPreTranslateMessageInEventLoop();

        ApplicationConfiguration.Initialize();

        using var form = new Form
        {
            Text = "WebGPT Xaml Island Private Fixture",
            Width = 945,
            Height = 760,
            StartPosition = FormStartPosition.Manual,
            Left = 80,
            Top = 80
        };

        var host = new IslandHostControl { Dock = DockStyle.Fill };
        form.Controls.Add(host);

        var root = new StackPanel { Spacing = 14, Padding = new Thickness(28) };
        root.Background = new SolidColorBrush(Windows.UI.Color.FromArgb(255, 245, 245, 245));

        var heading = new TextBlock { Text = "WebGPT XAML Island Private Fixture", FontSize = 26 };
        root.Children.Add(heading);

        var input = new Microsoft.UI.Xaml.Controls.TextBox { Header = "Input", FontSize = 20 };
        SetId(input, "fixture-input");
        root.Children.Add(input);

        var button = new Microsoft.UI.Xaml.Controls.Button { Content = "Background Action", FontSize = 20 };
        SetId(button, "fixture-button");
        root.Children.Add(button);

        var check = new Microsoft.UI.Xaml.Controls.CheckBox { Content = "Background Check", FontSize = 20 };
        SetId(check, "fixture-check");
        root.Children.Add(check);

        var combo = new Microsoft.UI.Xaml.Controls.ComboBox
        {
            Header = "Fixture Select",
            FontSize = 20,
            ItemsSource = new List<string> { "Red", "Green", "Blue" },
            SelectedIndex = 0
        };
        SetId(combo, "fixture-select");
        root.Children.Add(combo);

        var status = new TextBlock { Text = "idle", FontSize = 20 };
        SetId(status, "fixture-status");
        root.Children.Add(status);

        int clicks = 0;
        input.TextChanged += (_, _) => status.Text = "text=" + input.Text;
        button.Click += (_, _) => status.Text = "clicked-" + (++clicks);
        check.Checked += (_, _) => status.Text = "check=true";
        check.Unchecked += (_, _) => status.Text = "check=false";
        combo.SelectionChanged += (_, _) =>
        {
            if (combo.SelectedItem != null) status.Text = "selected=" + combo.SelectedItem;
        };

        host.Content = root;
        System.Windows.Forms.Application.Run(form);
        controller.ShutdownQueue();
        GC.KeepAlive(xamlApp);
    }
}
