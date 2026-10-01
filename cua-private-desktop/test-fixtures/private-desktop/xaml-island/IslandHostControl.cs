using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Media;
using System;
using System.ComponentModel;
using System.Windows.Forms;

namespace WebGPT.XamlIslandFixture;

internal sealed class IslandHostControl : System.Windows.Forms.Control
{
    private DesktopWindowXamlSource? _source;
    private Frame? _frame;
    private FrameworkElement? _content;

    [DesignerSerializationVisibility(DesignerSerializationVisibility.Hidden)]
    public FrameworkElement? Content
    {
        get => _content;
        set
        {
            _content = value;
            if (_frame != null) _frame.Content = value;
        }
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        if (_source == null) InitializeIsland();
        base.OnPaint(e);
    }

    protected override void OnResize(EventArgs e)
    {
        base.OnResize(e);
        _source?.SiteBridge.MoveAndResize(new Windows.Graphics.RectInt32(0, 0, Width, Height));
    }

    private void InitializeIsland()
    {
        _source = new DesktopWindowXamlSource();
        _source.Initialize(new WindowId((ulong)Handle));
        _source.SiteBridge.MoveAndResize(new Windows.Graphics.RectInt32(0, 0, Width, Height));
        _frame = new Frame();
        _source.Content = _frame;
        _source.SystemBackdrop = new DesktopAcrylicBackdrop();
        if (_content != null) _frame.Content = _content;
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing) _source?.Dispose();
        base.Dispose(disposing);
    }
}
