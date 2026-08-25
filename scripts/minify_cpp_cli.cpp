#include <minify/Minify.h>
#include <iostream>
#include <string>
int main(int argc, char** argv){
    if (argc != 3) return 2;
    std::string fmt = argv[1], input = argv[2];
    minify::Format f;
    if (fmt=="html") f=minify::Format::Html;
    else if (fmt=="css") f=minify::Format::Css;
    else if (fmt=="js") f=minify::Format::JavaScript;
    else if (fmt=="jsx") f=minify::Format::Jsx;
    else if (fmt=="json") f=minify::Format::Json;
    else if (fmt=="xml") f=minify::Format::Xml;
    else if (fmt=="svg") f=minify::Format::Svg;
    else return 2;
    std::string out, err;
    if (!minify::run(f, input, out, err)) { std::cout << "ERR:" << err << "\n"; return 0; }
    std::cout << out;
    return 0;
}
