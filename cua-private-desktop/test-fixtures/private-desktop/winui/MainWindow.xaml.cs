using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace winui_private_fixture;

public sealed partial class MainWindow : Window
{
    private int _clicks;

    public MainWindow()
    {
        InitializeComponent();
        FixtureStatus.Text = "idle";
    }

    private void FixtureInput_TextChanged(object sender, TextChangedEventArgs e)
    {
        if (FixtureStatus != null) FixtureStatus.Text = "text=" + FixtureInput.Text;
    }

    private void FixtureButton_Click(object sender, RoutedEventArgs e)
    {
        FixtureStatus.Text = "clicked-" + (++_clicks);
    }

    private void FixtureCheck_Changed(object sender, RoutedEventArgs e)
    {
        FixtureStatus.Text = "check=" + (FixtureCheck.IsChecked == true ? "true" : "false");
    }

    private void FixtureSelect_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (FixtureStatus == null || FixtureSelect.SelectedItem is not ComboBoxItem item) return;
        FixtureStatus.Text = "selected=" + (item.Content?.ToString() ?? "");
    }
}
