// Apple's on-device model (Foundation Models, macOS 27) on the app's requests.
//
// Reads a job file written by run_afm.py — the prompt, the genre list, and the
// images already prepared as the app prepares them — and answers each image
// in a fresh session, one at a time, printing one JSON line per request.
//
//   swiftc -O -parse-as-library main.swift -o afm && ./afm job.json
//
// Tags use guided generation with a schema that mirrors tagger::schema, in the
// same property order — main_subject before genre, which is load-bearing (the
// model settles what the photo is of before choosing a genre for it). It is
// built at run time (DynamicGenerationSchema) rather than with @Generable:
// the macro's plugin ships with Xcode, not the Command Line Tools, and the
// schema is the same either way.

import CoreGraphics
import Foundation
import FoundationModels
import ImageIO

/// The app's GENRES (models.rs). run_afm.py checks this against the source.
let genres = [
    "landscape", "wildlife", "portrait", "street", "macro", "astro", "architecture",
    "sports", "event", "still-life", "abstract",
]

/// `struct Tags { main_subject, subjects, conditions, genre (one of GENRES),
/// confidence }`, in that order.
func tagsSchema() throws -> GenerationSchema {
    let text = DynamicGenerationSchema(type: String.self)
    let root = DynamicGenerationSchema(
        name: "Tags",
        properties: [
            .init(name: "main_subject", schema: text),
            .init(name: "subjects", schema: DynamicGenerationSchema(arrayOf: text)),
            .init(name: "conditions", schema: DynamicGenerationSchema(arrayOf: text)),
            .init(name: "genre", schema: DynamicGenerationSchema(name: "Genre", anyOf: genres)),
            .init(name: "confidence", schema: DynamicGenerationSchema(type: Double.self)),
        ])
    return try GenerationSchema(root: root, dependencies: [])
}

struct Job: Decodable {
    let task: String  // "tag" or "describe"
    let prompt: String
    let images: [String]
    let genres: [String]
}

struct Line: Encodable {
    let image: String
    let latency: Double
    let text: String
    let prompt_n: Int?
    let cached_n: Int?
    let gen_n: Int?
    let error: String?
}

@main
struct AFM {
    static func main() async throws {
        let args = CommandLine.arguments
        if args.count > 1 && args[1] == "--availability" {
            let model = SystemLanguageModel.default
            print("availability: \(model.availability)")
            print("supports current locale: \(model.supportsLocale())")
            return
        }
        guard args.count > 1 else {
            FileHandle.standardError.write("usage: afm job.json | afm --availability\n".data(using: .utf8)!)
            exit(2)
        }
        let job = try JSONDecoder().decode(Job.self, from: Data(contentsOf: URL(fileURLWithPath: args[1])))
        if job.genres != genres {
            FileHandle.standardError.write("genre list differs from the app's: \(job.genres)\n".data(using: .utf8)!)
            exit(3)
        }
        let encoder = JSONEncoder()
        let schema = try tagsSchema()
        for image in job.images {
            let url = URL(fileURLWithPath: image)
            // A fresh session per photo: the context is small, and nothing
            // from one photo should colour the next.
            let session = LanguageModelSession()
            let start = Date()
            var line: Line
            do {
                if job.task == "tag" {
                    let response = try await session.respond(
                        schema: schema,
                        options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 256)
                    ) {
                        job.prompt
                        Attachment(imageURL: url)
                    }
                    line = Line(
                        image: image, latency: Date().timeIntervalSince(start),
                        text: response.content.jsonString,
                        prompt_n: response.usage.input.totalTokenCount,
                        cached_n: response.usage.input.cachedTokenCount,
                        gen_n: response.usage.output.totalTokenCount, error: nil)
                } else {
                    let response = try await session.respond(
                        options: GenerationOptions(temperature: 0.2, maximumResponseTokens: 512)
                    ) {
                        job.prompt
                        Attachment(imageURL: url)
                    }
                    line = Line(
                        image: image, latency: Date().timeIntervalSince(start), text: response.content,
                        prompt_n: response.usage.input.totalTokenCount,
                        cached_n: response.usage.input.cachedTokenCount,
                        gen_n: response.usage.output.totalTokenCount, error: nil)
                }
            } catch {
                line = Line(
                    image: image, latency: Date().timeIntervalSince(start), text: "", prompt_n: nil,
                    cached_n: nil, gen_n: nil, error: "\(error)")
            }
            print(String(data: try encoder.encode(line), encoding: .utf8)!)
            fflush(stdout)
        }
    }
}
