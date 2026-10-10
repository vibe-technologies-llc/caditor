# Shell

{command:model.shell} hollows a body, leaving walls of a thickness and opening the faces you
choose. Select the faces to open, then choose Shell.

While it is open, click faces in the view to open or close them; the opened faces show in the
selected colour. **Choose in the view** in the panel first opens the faces of the body selected
then. The panel lists them, each with a button to close it again (hovering one lights it in the
view), and sets the **Thickness**. Typing it previews the result before you press Enter.

A new shell starts from the thickness last used on one, kept between sessions (a thickness naming
a parameter the model lacks falls back to 1 mm).

Only flat faces can be opened. The walls grow inward from the body's faces, so its outside stays
as it was.
Faces leaning over an opening, such as a chamfer around it, keep their thickness up to the
opening; when the thickness would close the opening or cut such a wall thinner, the shell names
the face and asks for a smaller thickness instead.
A face narrower than the walls it lies between, such as a small chamfer, closes up and leaves the
cavity, its neighbours' walls meeting where it was. When two such faces lie side by side, the shell
names them and asks for a smaller thickness.
