using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;

public sealed class DynamicWpfEpochFixture : Window
{
    private readonly StackPanel panel;
    private readonly TextBlock status;
    private CheckBox target;
    private int phase;

    public DynamicWpfEpochFixture()
    {
        Title = "WebGPT Dynamic WPF Epoch Fixture";
        Width = 480;
        Height = 240;

        panel = new StackPanel { Margin = new Thickness(20) };
        status = new TextBlock { Name = "StatusLabel", Text = "phase=0" };
        panel.Children.Add(status);
        Content = panel;

        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(1200) };
        timer.Tick += delegate
        {
            phase++;
            status.Text = "phase=" + phase;
            if (phase == 1)
            {
                target = new CheckBox
                {
                    Name = "DynamicTarget",
                    Content = "Dynamic Target",
                    IsChecked = false
                };
                panel.Children.Add(target);
            }
            else if (phase == 2 && target != null)
            {
                target.IsChecked = true;
                target.Content = "Dynamic Target Ready";
            }
            else if (phase == 3 && target != null)
            {
                panel.Children.Remove(target);
                target = null;
            }
            else if (phase >= 4)
            {
                timer.Stop();
            }
        };
        Loaded += delegate { timer.Start(); };
    }

    [STAThread]
    public static void Main()
    {
        var app = new Application();
        app.Run(new DynamicWpfEpochFixture());
    }
}
