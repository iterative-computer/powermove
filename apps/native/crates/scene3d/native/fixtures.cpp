// Synthetic CAD fixtures generated independently by OpenCascade's writers.
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Builder.hxx>
#include <BRepTools.hxx>
#include <STEPCAFControl_Writer.hxx>
#include <IGESCAFControl_Writer.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <TopoDS_Compound.hxx>
#include <TopExp_Explorer.hxx>
#include <Quantity_Color.hxx>
#include <filesystem>
int main(int argc,char**argv){
  if(argc!=2)return 2;
  const std::filesystem::path root(argv[1]);
  Handle(TDocStd_Document) doc;
  XCAFApp_Application::GetApplication()->NewDocument("BinXCAF",doc);
  const auto shapes=XCAFDoc_DocumentTool::ShapeTool(doc->Main());
  const auto colors=XCAFDoc_DocumentTool::ColorTool(doc->Main());
  const auto body=BRepPrimAPI_MakeBox(20,30,10).Shape();
  const auto cap=BRepPrimAPI_MakeBox(gp_Pnt(0,0,10),20,30,2).Shape();
  TopoDS_Compound assembly;BRep_Builder builder;builder.MakeCompound(assembly);
  const auto assemblyLabel=shapes->AddShape(assembly,true);
  TDataStd_Name::Set(assemblyLabel,"Synthetic product");
  const auto bodyLabel=shapes->AddShape(body,false);
  const auto capLabel=shapes->AddShape(cap,false);
  TDataStd_Name::Set(bodyLabel,"Body");TDataStd_Name::Set(capLabel,"Cap");
  colors->SetColor(bodyLabel,Quantity_Color(0.2,0.4,0.8,Quantity_TOC_sRGB),XCAFDoc_ColorSurf);
  colors->SetColor(capLabel,Quantity_Color(0.2,0.8,0.4,Quantity_TOC_sRGB),XCAFDoc_ColorSurf);
  TopExp_Explorer face(body,TopAbs_FACE);
  if(!face.More())return 3;
  const auto faceLabel=shapes->AddSubShape(bodyLabel,face.Current());
  colors->SetColor(faceLabel,Quantity_Color(0.8,0.2,0.2,Quantity_TOC_sRGB),XCAFDoc_ColorSurf);
  shapes->AddComponent(assemblyLabel,bodyLabel,TopLoc_Location());
  shapes->AddComponent(assemblyLabel,capLabel,TopLoc_Location());
  shapes->UpdateAssemblies();
  STEPCAFControl_Writer step;step.SetColorMode(true);step.SetNameMode(true);
  if(!step.Transfer(doc,STEPControl_AsIs)||step.Write((root/"product.step").string().c_str())!=IFSelect_RetDone)return 4;
  IGESCAFControl_Writer iges;iges.SetColorMode(true);iges.SetNameMode(true);
  if(!iges.Transfer(doc)||!iges.Write((root/"product.iges").string().c_str()))return 5;
  TopoDS_Compound brep;builder.MakeCompound(brep);builder.Add(brep,body);builder.Add(brep,cap);
  if(!BRepTools::Write(brep,(root/"product.brep").string().c_str()))return 6;
  return 0;
}
