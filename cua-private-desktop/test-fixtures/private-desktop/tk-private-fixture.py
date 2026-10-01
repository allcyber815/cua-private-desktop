import tkinter as tk
from tkinter import ttk

root = tk.Tk()
root.title("WebGPT Tk Private Fixture")
root.geometry("945x760+80+80")

frame = ttk.Frame(root, padding=28)
frame.pack(fill="both", expand=True)

ttk.Label(frame, text="WebGPT Tk Private Fixture", font=("Segoe UI", 22)).pack(anchor="w", pady=(0, 20))

status_var = tk.StringVar(value="idle")
input_var = tk.StringVar(value="")
check_var = tk.BooleanVar(value=False)
select_var = tk.StringVar(value="Red")
clicks = {"n": 0}

ttk.Label(frame, text="Input").pack(anchor="w")
entry = ttk.Entry(frame, textvariable=input_var, width=48)
entry.pack(anchor="w", pady=(4, 16))

def on_input(*_):
    status_var.set("text=" + input_var.get())
input_var.trace_add("write", on_input)

def on_click():
    clicks["n"] += 1
    status_var.set("clicked-" + str(clicks["n"]))
button = ttk.Button(frame, text="Background Action", command=on_click)
button.pack(anchor="w", pady=(0, 16))

def on_check():
    status_var.set("check=" + ("true" if check_var.get() else "false"))
check = ttk.Checkbutton(frame, text="Background Check", variable=check_var, command=on_check)
check.pack(anchor="w", pady=(0, 16))

ttk.Label(frame, text="Fixture Select").pack(anchor="w")
combo = ttk.Combobox(frame, values=["Red", "Green", "Blue"], textvariable=select_var, state="readonly", width=30)
combo.pack(anchor="w", pady=(4, 16))
def on_select(_):
    status_var.set("selected=" + select_var.get())
combo.bind("<<ComboboxSelected>>", on_select)

status = ttk.Label(frame, textvariable=status_var, font=("Segoe UI", 14))
status.pack(anchor="w", pady=(16, 0))

root.mainloop()
