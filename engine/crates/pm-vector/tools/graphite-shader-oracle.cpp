// Extract Graphite WGSL and MSL with Skia's own compiler. Build against
// chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5 with Graphite enabled.
// Usage: oracle vertex|fragment input.sksl output-prefix [force-f32]
#include "src/sksl/SkSLCompiler.h"
#include "src/sksl/SkSLGraphiteModules.h"
#include "src/sksl/SkSLProgramSettings.h"
#include "src/sksl/SkSLStringStream.h"
#include "src/sksl/SkSLUtil.h"
#include "src/sksl/ir/SkSLProgram.h"
#include "src/sksl/codegen/SkSLMetalCodeGenerator.h"
#include "src/sksl/codegen/SkSLWGSLCodeGenerator.h"
#include <fstream>
#include <iostream>
#include <iterator>

int main(int argc, char** argv) {
    if (argc < 4 || argc > 5) return 2;
    const std::string stage = argv[1];
    if (stage != "vertex" && stage != "fragment") return 2;
    std::ifstream input(argv[2]);
    if (!input) return 2;
    const std::string source((std::istreambuf_iterator<char>(input)), {});
    SkSL::Loader::SetGraphiteModuleData(SkSL::Loader::GetGraphiteModules());
    SkSL::Compiler compiler;
    SkSL::ProgramSettings settings;
    settings.fForceNoRTFlip = true;
    settings.fSharpenTextures = true;
    settings.fForceHighPrecision = argc == 5;
    settings.fMaxVersionAllowed = SkSL::Version::k300;
    auto program = compiler.convertProgram(stage == "vertex"
        ? SkSL::ProgramKind::kGraphiteVertex : SkSL::ProgramKind::kGraphiteFragment,
        source, settings);
    if (!program) { std::cerr << compiler.errorText(); return 1; }
    SkSL::ShaderCaps caps;
    caps.fInfinitySupport = false; // DawnCaps::initShaderCaps.
    caps.fShaderDerivativeSupport = true;
    SkSL::StringStream wgsl, msl;
    if (!SkSL::ToWGSL(*program, &caps, wgsl) || !SkSL::ToMetal(*program, &caps, msl)) {
        std::cerr << compiler.errorText(); return 1;
    }
    std::ofstream(std::string(argv[3]) + ".wgsl") << wgsl.str();
    std::ofstream(std::string(argv[3]) + ".metal") << msl.str();
}
