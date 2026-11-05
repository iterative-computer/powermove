// Native OCCT 7.8+ XDE importer. Linked libraries retain their LGPL-2.1
// obligations; ship their license/source-offer with the signed helper bundle.
#include <BRep_Builder.hxx>
#include <BRepBndLib.hxx>
#include <BRepTools.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <Bnd_Box.hxx>
#include <IGESCAFControl_Reader.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <RWGltf_CafWriter.hxx>
#include <RWMesh_CoordinateSystem.hxx>
#include <TColStd_IndexedDataMapOfStringString.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDocStd_Document.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <TopoDS_Shape.hxx>
#include <Standard_Failure.hxx>
#include <algorithm>
#include <cmath>
#include <string>

int main(int argc, char** argv) {
  if (argc != 6) return 2;
  try {
    const std::string format(argv[1]);
    const double linear = std::stod(argv[4]), angular = std::stod(argv[5]);
    if (!std::isfinite(linear) || linear <= 0 || !std::isfinite(angular) || angular <= 0) return 2;
    Handle(TDocStd_Document) doc;
    XCAFApp_Application::GetApplication()->NewDocument("BinXCAF", doc);
    if (format == "step") {
      STEPCAFControl_Reader reader;
      reader.SetColorMode(true); reader.SetNameMode(true); reader.SetLayerMode(true);
      if (reader.ReadFile(argv[2]) != IFSelect_RetDone || !reader.Transfer(doc)) return 3;
    } else if (format == "iges") {
      IGESCAFControl_Reader reader;
      reader.SetColorMode(true); reader.SetNameMode(true); reader.SetLayerMode(true);
      if (reader.ReadFile(argv[2]) != IFSelect_RetDone || !reader.Transfer(doc)) return 3;
    } else if (format == "brep") {
      TopoDS_Shape shape; BRep_Builder builder;
      if (!BRepTools::Read(shape, argv[2], builder) || shape.IsNull()) return 3;
      XCAFDoc_DocumentTool::ShapeTool(doc->Main())->AddShape(shape, false);
    } else return 2;
    const auto shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
    TDF_LabelSequence roots; shapes->GetFreeShapes(roots);
    if (roots.IsEmpty()) return 4;
    Bnd_Box bounds;
    for (int i = 1; i <= roots.Length(); ++i) BRepBndLib::Add(shapes->GetShape(roots.Value(i)), bounds);
    if (bounds.IsVoid() || bounds.IsOpen()) return 4;
    double x0,y0,z0,x1,y1,z1; bounds.Get(x0,y0,z0,x1,y1,z1);
    const double size = std::max({x1-x0,y1-y0,z1-z0});
    if (!(size > 0) || !std::isfinite(size)) return 4;
    for (int i = 1; i <= roots.Length(); ++i) {
      BRepMesh_IncrementalMesh mesh(shapes->GetShape(roots.Value(i)), size*linear, false, angular, true);
      if (!mesh.IsDone()) return 5;
    }
    RWGltf_CafWriter writer(argv[3], true);
    // CAD source coordinates are Z-up, millimetres; output is glTF Y-up.
    writer.ChangeCoordinateSystemConverter().SetInputCoordinateSystem(RWMesh_CoordinateSystem_Zup);
    writer.ChangeCoordinateSystemConverter().SetOutputCoordinateSystem(RWMesh_CoordinateSystem_Yup);
    writer.ChangeCoordinateSystemConverter().SetInputLengthUnit(0.001);
    writer.ChangeCoordinateSystemConverter().SetOutputLengthUnit(1.0);
    writer.SetMergeFaces(false);
    TColStd_IndexedDataMapOfStringString metadata;
    if (!writer.Perform(doc, metadata, Message_ProgressRange())) return 6;
    return 0;
  } catch (const Standard_Failure&) { return 7; }
    catch (const std::exception&) { return 8; }
}
